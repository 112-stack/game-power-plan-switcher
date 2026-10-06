#requires -Version 5.1
<# Isolated kernel singleton/pipe integration test. No GUI, Slint event loop,
   engine, guardian, startup registration or power-plan mutation is invoked.
   The crash case terminates ONLY a probe Process object created by this script.
   Reports and the fresh NN6_TEST_DATA_DIR remain in work for inspection. #>
param(
    [Parameter(Mandatory=$true)][string]$Exe,
    [string]$ReportPath = (Join-Path $PSScriptRoot 'gui-singleton-verification.json')
)
$ErrorActionPreference='Stop'
$exePath=(Resolve-Path -LiteralPath $Exe).ProviderPath
$reportPath=[IO.Path]::GetFullPath($ReportPath)
[IO.Directory]::CreateDirectory((Split-Path -Parent $reportPath))|Out-Null
$testRoot=Join-Path $PSScriptRoot ('gui-singleton-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($testRoot)|Out-Null
$owned=New-Object 'System.Collections.Generic.List[object]'
$checks=New-Object 'System.Collections.Generic.List[object]'
$keptMutex=$null
$failure=$null
function Assert-Check([string]$Name,[bool]$Passed,[object]$Evidence){
    $checks.Add([pscustomobject]@{name=$Name;passed=$Passed;evidence=$Evidence})
    if(-not $Passed){throw "Failed: $Name"}
}
function Start-Probe([string]$Name,[int]$HoldMs){
    $probeReport=Join-Path $testRoot ($Name+'.json')
    # Windows paths cannot contain quotes. No shell or argument expansion runs.
    if($probeReport.Contains('"')){throw 'Invalid report path'}
    $info=New-Object Diagnostics.ProcessStartInfo
    $info.FileName=$exePath
    $info.Arguments='--gui-instance-probe "'+$probeReport+'" '+$HoldMs
    $info.WorkingDirectory=Split-Path -Parent $exePath
    $info.UseShellExecute=$false
    $info.CreateNoWindow=$true
    $info.WindowStyle=[Diagnostics.ProcessWindowStyle]::Hidden
    $info.EnvironmentVariables['NN6_TEST_DATA_DIR']=$testRoot
    $process=New-Object Diagnostics.Process
    $process.StartInfo=$info
    if(-not $process.Start()){throw "Could not start isolated probe $Name"}
    $item=[pscustomobject]@{Name=$Name;Process=$process;Report=$probeReport;Pid=$process.Id}
    $owned.Add($item)
    return $item
}
function Read-Probe($Item){
    if(-not [IO.File]::Exists($Item.Report)){return $null}
    try{return ([IO.File]::ReadAllText($Item.Report)|ConvertFrom-Json)}catch{return $null}
}
function Wait-ProbeReport($Item,[int]$TimeoutMs=12000){
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do{
        $data=Read-Probe $Item
        if($null -ne $data){return $data}
        if($Item.Process.HasExited){
            # The child may atomically publish its report between the first
            # read and HasExited. After exit, its completed write is observable.
            $data=Read-Probe $Item
            if($null -ne $data){return $data}
            throw "Probe $($Item.Name) exited $($Item.Process.ExitCode) without its report"
        }
        Start-Sleep -Milliseconds 40
    }while([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $($Item.Name) report"
}
function Wait-ProbeExit($Item,[int]$TimeoutMs=20000){
    if(-not $Item.Process.WaitForExit($TimeoutMs)){throw "Probe $($Item.Name) did not exit"}
    if($Item.Process.ExitCode -ne 0){throw "Probe $($Item.Name) exited $($Item.Process.ExitCode)"}
    return (Read-Probe $Item)
}
try{
    $batch=@()
    # Launch all 30 without waiting for an owner or initializing a GUI first.
    for($index=0;$index -lt 30;$index++){$batch+=Start-Probe ('parallel-'+$index.ToString('00')) 15000}
    $initial=@($batch|ForEach-Object{Wait-ProbeReport $_})
    $owners=@($initial|Where-Object role -eq 'primary')
    $duplicates=@($initial|Where-Object role -eq 'forwarded')
    Assert-Check '30 concurrent launches select exactly one owner' ($owners.Count -eq 1 -and $duplicates.Count -eq 29) @{owners=$owners.Count;forwarded=$duplicates.Count}
    Assert-Check 'All launches share the isolated namespace' (@($initial.scope_key|Sort-Object -Unique).Count -eq 1) $owners[0].scope_key
    Assert-Check 'Duplicates finish within the bounded startup handoff' (@($duplicates|Where-Object elapsed_ms -gt 10000).Count -eq 0) @($duplicates.elapsed_ms)
    $ownerItem=@($batch|Where-Object Pid -eq $owners[0].pid)[0]

    $pipeName='NN6.PowerPlan.GUI.v1.'+$owners[0].scope_key
    $pipe=[IO.Pipes.NamedPipeClientStream]::new('.',$pipeName,[IO.Pipes.PipeDirection]::InOut,[IO.Pipes.PipeOptions]::Asynchronous,[Security.Principal.TokenImpersonationLevel]::Identification)
    try{
        $pipe.Connect(2000)
        $invalid=[Text.Encoding]::ASCII.GetBytes('NN6NOPE1')
        if(-not $pipe.WriteAsync($invalid,0,8).Wait(2000)){throw 'Malformed request write timed out'}
        $answer=New-Object byte[] 8
        $offset=0
        while($offset -lt 8){
            $read=$pipe.ReadAsync($answer,$offset,8-$offset)
            if(-not $read.Wait(2000)){throw 'Malformed request response timed out'}
            if($read.Result -eq 0){throw 'Malformed request connection closed before response'}
            $offset+=$read.Result
        }
        $response=[Text.Encoding]::ASCII.GetString($answer)
        Assert-Check 'Malformed command is rejected' ($response -eq 'NN6ERR01') $response
        $receipt=[Text.Encoding]::ASCII.GetBytes('NN6DONE1')
        if(-not $pipe.WriteAsync($receipt,0,8).Wait(2000)){throw 'Malformed response receipt timed out'}
    }finally{$pipe.Dispose()}
    foreach($item in $batch){$null=Wait-ProbeExit $item}
    $ownerFinal=Read-Probe $ownerItem
    Assert-Check 'Owner received all 29 SHOW requests and no malformed command' ($ownerFinal.phase -eq 'closed' -and $ownerFinal.show_requests -eq 29) $ownerFinal

    $crash=Start-Probe 'crash-owner' 30000
    $crashReport=Wait-ProbeReport $crash
    Assert-Check 'Fresh owner acquires after clean shutdown' ($crashReport.role -eq 'primary') $crashReport
    # Keep the kernel object open but unowned. This makes the next launch observe
    # WAIT_ABANDONED rather than merely create a new object after the owner exits.
    $keptMutex=[Threading.Mutex]::OpenExisting('Global\NN6.PowerPlan.GUI.v1.'+$crashReport.scope_key)
    Assert-Check 'Crash fixture is the exact process started by this harness' ($crash.Process.Id -eq $crashReport.pid -and -not $crash.Process.HasExited) $crashReport.pid
    $crash.Process.Kill()
    if(-not $crash.Process.WaitForExit(5000)){throw 'Owned crash fixture did not terminate'}
    $replacement=Start-Probe 'after-crash' 300
    $replacementReport=Wait-ProbeReport $replacement
    Assert-Check 'Abandoned mutex recovers to a new owner' ($replacementReport.role -eq 'primary' -and $replacementReport.recovered_abandoned) $replacementReport
    $keptMutex.Dispose();$keptMutex=$null
    $null=Wait-ProbeExit $replacement
    $final=Start-Probe 'after-recovery-clean-exit' 150
    $finalReport=Wait-ProbeExit $final
    Assert-Check 'Ownership releases after recovery exits cleanly' ($finalReport.role -eq 'primary' -and $finalReport.phase -eq 'closed') $finalReport
}catch{$failure=$_.Exception.Message}
finally{
    if($null -ne $keptMutex){$keptMutex.Dispose()}
    # Never enumerate or terminate other NN6/game processes. Only retained probe
    # Process handles created above are eligible for failure cleanup.
    foreach($item in $owned){
        try{if(-not $item.Process.HasExited){$item.Process.Kill();$null=$item.Process.WaitForExit(5000)}}catch{}
        $item.Process.Dispose()
    }
    $result=[ordered]@{
        checked_at_utc=[DateTime]::UtcNow.ToString('o');exe=$exePath
        sha256=(Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash
        test_data_dir=$testRoot;scope='Isolated kernel mutex/pipe probe only; no GUI, engine, power or startup actions.'
        passed=($null -eq $failure);error=$failure;checks=@($checks.ToArray())
    }
    $result|ConvertTo-Json -Depth 8|Set-Content -LiteralPath $reportPath -Encoding UTF8
}
if($null -ne $failure){throw "$failure; report: $reportPath"}
Write-Output "Passed $($checks.Count) singleton checks. Report: $reportPath"
