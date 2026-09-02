const fs = require('fs');
const path = require('path');

const iconsDir = path.join(__dirname, 'src-tauri', 'icons');
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

// Minimal valid PNG byte sequence (1x1 green pixel)
const pngBytes = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAAF0lEQVRYR2Nk+M9AF8CNRtFoFI1GUQIAOwcBAfN+N5oAAAAASUVORK5CYII=',
  'base64'
);

// Create valid ICO header for PNG payload
const icoHeader = Buffer.alloc(22);
icoHeader.writeUInt16LE(0, 0); // Reserved
icoHeader.writeUInt16LE(1, 2); // Type 1 (.ICO)
icoHeader.writeUInt16LE(1, 4); // 1 Image

icoHeader.writeUInt8(32, 6);  // Width 32
icoHeader.writeUInt8(32, 7);  // Height 32
icoHeader.writeUInt8(0, 8);   // Color count
icoHeader.writeUInt8(0, 9);   // Reserved
icoHeader.writeUInt16LE(1, 10); // Color planes
icoHeader.writeUInt16LE(32, 12); // Bits per pixel
icoHeader.writeUInt32LE(pngBytes.length, 14); // Image size in bytes
icoHeader.writeUInt32LE(22, 18); // Offset to image data (22 bytes)

const validIco = Buffer.concat([icoHeader, pngBytes]);

fs.writeFileSync(path.join(iconsDir, 'icon.ico'), validIco);
fs.writeFileSync(path.join(iconsDir, '32x32.png'), pngBytes);
fs.writeFileSync(path.join(iconsDir, '128x128.png'), pngBytes);
fs.writeFileSync(path.join(iconsDir, '128x128@2x.png'), pngBytes);
fs.writeFileSync(path.join(iconsDir, 'icon.png'), pngBytes);

console.log('Successfully created valid ICO & PNG icons');
