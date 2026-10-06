#requires -Version 5.1
# Disposable integration run under the ordinary interactive Windows token.
param([Parameter(Mandatory=$true)][string]$Executable)
$ErrorActionPreference='Stop'
$Executable=(Resolve-Path -LiteralPath $Executable).Path
$project=Split-Path $PSScriptRoot -Parent
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$taskName='GamePowerPlan-RainbowSix-'+$sid
$task=Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
$wasEnabled=$task -and $task.Settings.Enabled
$wasRunning=$task -and $task.State -eq 'Running'
$original=([regex]::Match((& powercfg /getactivescheme | Out-String),'[0-9a-fA-F-]{36}')).Value
$runKey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$oldRun=(Get-ItemProperty -LiteralPath $runKey -Name NN6PowerPlanNative -ErrorAction SilentlyContinue).NN6PowerPlanNative
$testDir=Join-Path $PSScriptRoot ('native-live-'+[guid]::NewGuid().ToString('N'))
$resultFile=Join-Path $PSScriptRoot 'native-live-results.json'
$wrapperFile=Join-Path $PSScriptRoot 'native-live-wrapper.json'
$outcome=@{passed=$false;error=$null;originalPlan=$original;testDirectory=$testDir;user=[Security.Principal.WindowsIdentity]::GetCurrent().Name}
try {
    if(Get-Process -Name RainbowSix,RainbowSixSiege,RainbowSix_DX11 -ErrorAction SilentlyContinue){throw 'Close the real game before live testing.'}
    if($task){Disable-ScheduledTask -TaskName $taskName|Out-Null;Stop-ScheduledTask -TaskName $taskName}
    $deadline=[DateTime]::UtcNow.AddSeconds(15)
    do {Start-Sleep -Milliseconds 200;$ready=$true;$lock=$null;try {$lock=[IO.File]::Open((Join-Path $env:LOCALAPPDATA 'GamePowerPlan\monitor.lock'),'OpenOrCreate','ReadWrite','None')}catch{$ready=$false}finally{if($lock){$lock.Dispose()}}}while(-not $ready -and [DateTime]::UtcNow -lt $deadline)
    if(-not $ready){throw 'Another monitor still owns monitor.lock.'}
    $env:NN6_TEST_DATA_DIR=$testDir
    $process=Start-Process -FilePath $Executable -ArgumentList @('--exercise',('"'+$resultFile+'"'),'--live') -WindowStyle Hidden -PassThru
    if(-not $process.WaitForExit(180000)){Stop-Process -Id $process.Id -Force;throw 'Native live exercise timed out.'}
    $report=[IO.File]::ReadAllText($resultFile)|ConvertFrom-Json
    if($report.error){throw $report.error}
    $outcome.passed=$true
} catch { $outcome.error=$_.Exception.Message }
finally {
    # Stop only owned test children if an early failure left them alive.
    Get-CimInstance Win32_Process | Where-Object {$_.ExecutablePath -and $_.ExecutablePath.StartsWith($testDir,[StringComparison]::OrdinalIgnoreCase)} | ForEach-Object {Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue}
    if($oldRun){Set-ItemProperty -LiteralPath $runKey -Name NN6PowerPlanNative -Value $oldRun}
    else{Remove-ItemProperty -LiteralPath $runKey -Name NN6PowerPlanNative -ErrorAction SilentlyContinue}
    $current=([regex]::Match((& powercfg /getactivescheme | Out-String),'[0-9a-fA-F-]{36}')).Value
    if($original -and $current -ne $original){& powercfg /setactive $original | Out-Null}
    if($wasEnabled){Enable-ScheduledTask -TaskName $taskName|Out-Null}
    if($wasRunning){Start-ScheduledTask -TaskName $taskName}
    $outcome.finalPlan=(& powercfg /getactivescheme | Out-String).Trim()
    $outcome.originalTaskEnabled=[bool](Get-ScheduledTask -TaskName $taskName).Settings.Enabled
    $outcome|ConvertTo-Json|Set-Content -LiteralPath $wrapperFile -Encoding UTF8
}
