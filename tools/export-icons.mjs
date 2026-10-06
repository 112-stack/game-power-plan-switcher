/**
 * Export NN6's editable SVG master using sharp's SVG renderer.
 * Usage: node tools/export-icons.mjs [path-to-sharp-package]
 * The optional argument supports a bundled/local sharp without a global install.
 * No artwork is downloaded; all sizes are rendered from assets/icon.svg.
 */
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs/promises';
import path from 'node:path';

const sourceRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const assets = path.join(sourceRoot, 'assets');
const require = createRequire(path.join(sourceRoot, 'package.json'));
const sharp = process.argv[2] ? require(path.resolve(process.argv[2])) : require('sharp');
const svg = await fs.readFile(path.join(assets, 'icon.svg'));

// Render directly at each destination size rather than resizing an ICO frame.
const render = (size) => sharp(svg, { density: 96 * size / 256 })
  .resize(size, size)
  .png({ compressionLevel: 9, adaptiveFiltering: true })
  .toBuffer();

await fs.writeFile(path.join(assets, 'icon.png'), await render(512));
await fs.writeFile(path.join(assets, 'icon-1024.png'), await render(1024));

// Windows Vista+ supports lossless PNG payloads in ICO containers.
const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
const frames = await Promise.all(sizes.map(render));
const directory = Buffer.alloc(6 + sizes.length * 16);
directory.writeUInt16LE(1, 2);
directory.writeUInt16LE(sizes.length, 4);
let offset = directory.length;
for (const [index, size] of sizes.entries()) {
  const entry = 6 + index * 16;
  directory[entry] = size === 256 ? 0 : size;
  directory[entry + 1] = directory[entry];
  directory.writeUInt16LE(1, entry + 4);
  directory.writeUInt16LE(32, entry + 6);
  directory.writeUInt32LE(frames[index].length, entry + 8);
  directory.writeUInt32LE(offset, entry + 12);
  offset += frames[index].length;
}
await fs.writeFile(path.join(assets, 'NN6.ico'), Buffer.concat([directory, ...frames]));

// ICNS PNG element types. This is an export asset, not a macOS application build.
const icnsFrames = await Promise.all([
  ['icp4', 16], ['icp5', 32], ['icp6', 64], ['ic07', 128],
  ['ic08', 256], ['ic09', 512], ['ic10', 1024],
].map(async ([type, size]) => {
  const png = await render(size);
  const header = Buffer.alloc(8);
  header.write(type, 0, 4, 'ascii');
  header.writeUInt32BE(png.length + 8, 4);
  return Buffer.concat([header, png]);
}));
const icnsHeader = Buffer.alloc(8);
icnsHeader.write('icns', 0, 4, 'ascii');
icnsHeader.writeUInt32BE(8 + icnsFrames.reduce((sum, frame) => sum + frame.length, 0), 4);
await fs.writeFile(path.join(assets, 'NN6.icns'), Buffer.concat([icnsHeader, ...icnsFrames]));

console.log(`Exported icon.png (512), icon-1024.png, NN6.ico (${sizes.join(', ')}), NN6.icns (16–1024) from icon.svg.`);
