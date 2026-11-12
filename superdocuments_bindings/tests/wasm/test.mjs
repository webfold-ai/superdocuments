// Tests for the WebAssembly build, against real documents. Run `scripts/build-wasm.sh` first, then
// `node tests/wasm/test.mjs`. Inputs are the repository's own fixtures (../../tests/fixtures); the EPUB
// and the hostile inputs are built here.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const require = createRequire(import.meta.url);
const sd = require("../../dist/superdocuments_bindings.js");
const fixtures = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "tests", "fixtures");
const fixture = (name) => new Uint8Array(readFileSync(join(fixtures, name)));
const text = (s) => new TextEncoder().encode(s);

// A minimal ZIP writer: entries are [name, bytes, method] with method 0 (stored) or 8 (deflate).
function zip(entries) {
  const chunks = [];
  const central = [];
  let offset = 0;
  const u16 = (n) => { const b = Buffer.alloc(2); b.writeUInt16LE(n); return b; };
  const u32 = (n) => { const b = Buffer.alloc(4); b.writeUInt32LE(n >>> 0); return b; };
  for (const [name, data, method = 0] of entries) {
    const raw = Buffer.from(data);
    const body = method === 8 ? zlib.deflateRawSync(raw, { level: 9 }) : raw;
    const nm = Buffer.from(name);
    const crc = zlib.crc32(raw);
    const local = Buffer.concat([u32(0x04034b50), u16(20), u16(0), u16(method), u16(0), u16(0x21), u32(crc), u32(body.length), u32(raw.length), u16(nm.length), u16(0), nm, body]);
    central.push(Buffer.concat([u32(0x02014b50), u16(20), u16(20), u16(0), u16(method), u16(0), u16(0x21), u32(crc), u32(body.length), u32(raw.length), u16(nm.length), u16(0), u16(0), u16(0), u16(0), u32(0), u32(offset), nm]));
    chunks.push(local);
    offset += local.length;
  }
  const cd = Buffer.concat(central);
  const end = Buffer.concat([u32(0x06054b50), u16(0), u16(0), u16(entries.length), u16(entries.length), u32(cd.length), u32(offset), u16(0)]);
  return new Uint8Array(Buffer.concat([...chunks, cd, end]));
}

const epub = zip([
  ["mimetype", "application/epub+zip"],
  ["META-INF/container.xml", '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>'],
  ["OEBPS/content.opf", '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">x</dc:identifier><dc:title>Fixture Book</dc:title></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>'],
  ["OEBPS/c1.xhtml", '<html xmlns="http://www.w3.org/1999/xhtml"><body><h1>Chapter One</h1><p>Call me <b>Ishmael</b>.</p></body></html>'],
]);

let n = 0;
const ok = (name, fn) => { fn(); n += 1; };

// ---- real documents ----
ok("docx", () => {
  const md = sd.convert(fixture("office/basic.docx"), "docx");
  assert.match(md, /# Quarterly Report/);
  assert.match(md, /\*\*Bold part\*\*/);
  assert.match(md, /- First bullet/);
});
ok("xlsx", () => {
  const md = sd.convert(fixture("office/basic.xlsx"), "xlsx");
  assert.match(md, /## Costs/);
  assert.match(md, /\| A & B \| 2026-11-07 \| 1234\.5 \| TRUE \|/);
});
ok("pptx", () => {
  const md = sd.convert(fixture("office/basic.pptx"), "pptx");
  assert.match(md, /## Slide 1:/);
  assert.match(md, /\*\*Notes:\*\* Speaker note here/);
});
ok("odt and ods", () => {
  assert.match(sd.convert(fixture("office/basic.odt"), "odt"), /# Overview/);
  assert.match(sd.convert(fixture("office/basic.ods"), "ods"), /## Costs/);
});
ok("rtf", () => {
  const md = sd.convert(fixture("office/rich.rtf"), "rtf");
  assert.match(md, /\[the site\]\(https:\/\/example\.com\/a\?x=1&y=2\)/);
  assert.match(md, /Footnote body & text\./);
});
ok("legacy xls (the in-repo calamine crate, in wasm)", () => {
  assert.match(sd.convert(fixture("xls/lo-sheets.xls"), "xls"), /\|/);
});
ok("epub built here", () => {
  const md = sd.convert(epub, "epub");
  assert.match(md, /Chapter One/);
  assert.match(md, /Ishmael/);
});
ok("csv, text and html", () => {
  assert.match(sd.convert(text("name,qty\nbolt,3\n"), "csv"), /\| bolt \| 3 \|/);
  assert.equal(sd.convert(text("# just markdown"), "md"), "# just markdown");
  assert.match(sd.convert(text("<html><body><h1>Title</h1></body></html>"), "html"), /# Title/);
});
ok("extension is normalised", () => {
  assert.equal(sd.convert(fixture("office/basic.docx"), ".DOCX"), sd.convert(fixture("office/basic.docx"), "docx"));
});

// ---- pdf, in process ----
ok("pdf", () => {
  const pdf = fixture("reportlab-invoice.pdf");
  const md = sd.convert(pdf, "pdf");
  assert.match(md, /# Invoice Fixture 2025/);
  assert.match(md, /INV-2048/);
  assert.equal(md, sd.pdfToMarkdown(pdf));
  const pages = sd.pdfPages(pdf);
  assert.equal(pages.length, 1);
  assert.match(pages[0], /Total amount: \$1,234\.50/);
});
ok("a scanned pdf gets no OCR text", () => {
  const out = sd.convert(fixture("scanned-text.pdf"), "pdf");
  assert.doesNotMatch(out, /Scanned fixture|OCR works/);
});

// ---- worksheets ----
ok("worksheets", () => {
  const sheets = JSON.parse(sd.worksheets(fixture("office/basic.xlsx")));
  assert.deepEqual(sheets.map((s) => s.name), ["Costs", "Revenue"]);
  assert.deepEqual(sheets[0].rows, [["x", "y"], ["1", "2"]]);
});

// ---- hostile input: an Error, never a hang; and the module still works afterwards ----
const stillWorks = () => assert.match(sd.convert(text("a,b\n1,2\n"), "csv"), /\| a \| b \|/);
const rejects = (data, ext) => {
  const t0 = Date.now();
  assert.throws(() => sd.convert(data, ext), Error, `${ext} should have been rejected`);
  assert.ok(Date.now() - t0 < 20000, `${ext} took too long to reject`);
  stillWorks();
};
ok("empty input", () => {
  for (const ext of ["docx", "xlsx", "pptx", "odt", "ods", "epub", "xls", "xlsb", "rtf", "pdf"]) rejects(new Uint8Array(), ext);
});
ok("truncated archives", () => {
  for (const [name, ext] of [["office/basic.docx", "docx"], ["office/basic.xlsx", "xlsx"], ["office/basic.pptx", "pptx"], ["office/basic.odt", "odt"]]) {
    const data = fixture(name);
    for (const cut of [4, data.length >> 1, data.length - 10]) rejects(data.slice(0, cut), ext);
  }
});
ok("wrong extension where the format has a header check", () => {
  const docx = fixture("office/basic.docx");
  for (const ext of ["xls", "pdf", "rtf", "odt", "ods"]) rejects(docx, ext);
});
ok("a valid zip given to the wrong ooxml/epub converter is empty, not a crash", () => {
  const docx = fixture("office/basic.docx");
  for (const ext of ["xlsx", "pptx", "epub"]) {
    try { assert.equal(sd.convert(docx, ext).trim(), ""); } catch (e) { assert.ok(e instanceof Error); }
  }
  stillWorks();
});
ok("random bytes", () => {
  const junk = Uint8Array.from({ length: 4096 }, (_, i) => (i * 131 + 7) % 256);
  for (const ext of ["docx", "xlsx", "pptx", "odt", "epub", "xls", "xlsb", "pdf"]) rejects(junk, ext);
});
ok("a 400 MB zip bomb is refused quickly", () => {
  const bomb = zip([["[Content_Types].xml", "<Types/>"], ["word/document.xml", Buffer.alloc(400 * 1024 * 1024), 8]]);
  assert.ok(bomb.length < 5 * 1024 * 1024, `bomb is ${bomb.length} bytes`);
  rejects(bomb, "docx");
});
ok("worksheets on junk", () => {
  assert.throws(() => sd.worksheets(text("not a zip")), Error);
  stillWorks();
});

// ---- meta ----
ok("extensions and version", () => {
  const exts = sd.extensions();
  for (const e of ["docx", "xlsx", "pptx", "epub", "rtf", "csv", "pdf"]) assert.ok(exts.includes(e), e);
  assert.match(sd.version(), /^\d+\.\d+\.\d+/);
});
console.log(`wasm: ${n} test groups passed`);
