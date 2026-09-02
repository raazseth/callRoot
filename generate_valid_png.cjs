const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

// Create a valid 32x32 RGBA PNG file
function createPng(width, height) {
  const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);

  // IHDR chunk
  const ihdrData = Buffer.alloc(13);
  ihdrData.writeUInt32BE(width, 0);
  ihdrData.writeUInt32BE(height, 4);
  ihdrData.writeUInt8(8, 8);  // 8 bits per channel
  ihdrData.writeUInt8(6, 9);  // RGBA color type
  ihdrData.writeUInt8(0, 10); // compression
  ihdrData.writeUInt8(0, 11); // filter
  ihdrData.writeUInt8(0, 12); // interlace
  const ihdrChunk = createChunk('IHDR', ihdrData);

  // Raw pixel data: 32x32 RGBA green pixels
  const rawScanlines = [];
  for (let y = 0; y < height; y++) {
    const scanline = Buffer.alloc(1 + width * 4);
    scanline[0] = 0; // Filter type None
    for (let x = 0; x < width; x++) {
      const idx = 1 + x * 4;
      scanline[idx] = 16;     // R
      scanline[idx + 1] = 185; // G (emerald #10b981)
      scanline[idx + 2] = 129; // B
      scanline[idx + 3] = 255; // A
    }
    rawScanlines.push(scanline);
  }

  const idatRaw = Buffer.concat(rawScanlines);
  const idatCompressed = zlib.deflateSync(idatRaw);
  const idatChunk = createChunk('IDAT', idatCompressed);

  // IEND chunk
  const iendChunk = createChunk('IEND', Buffer.alloc(0));

  return Buffer.concat([signature, ihdrChunk, idatChunk, iendChunk]);
}

function createChunk(type, data) {
  const len = data.length;
  const buf = Buffer.alloc(8 + len + 4);
  buf.writeUInt32BE(len, 0);
  buf.write(type, 4, 4, 'ascii');
  data.copy(buf, 8);

  const crc = crc32(buf.subarray(4, 8 + len));
  buf.writeInt32BE(crc, 8 + len);
  return buf;
}

// CRC32 implementation
function crc32(buf) {
  let crc = 0xffffffff;
  for (let i = 0; i < buf.length; i++) {
    crc ^= buf[i];
    for (let j = 0; j < 8; j++) {
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) | 0;
}

const iconsDir = path.join(__dirname, 'src-tauri', 'icons');
if (!fs.existsSync(iconsDir)) {
  fs.mkdirSync(iconsDir, { recursive: true });
}

const validPng = createPng(32, 32);
fs.writeFileSync(path.join(iconsDir, '32x32.png'), validPng);
fs.writeFileSync(path.join(iconsDir, '128x128.png'), createPng(128, 128));
fs.writeFileSync(path.join(iconsDir, 'icon.png'), createPng(128, 128));

console.log('Successfully generated valid RGBA PNG icons');
