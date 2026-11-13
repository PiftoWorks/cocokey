// Writes THIRD-PARTY-NOTICES.txt: the license texts of everything built into
// CoCoKey for Windows (Rust crates and the JavaScript that ends up in the
// window), followed by CoCoKey's own license.
//
//   node tools/third-party-notices.mjs
//
// Run it again whenever Cargo.lock or package-lock.json changes; a test in
// src-tauri (notices::tests) fails when a package in Cargo.lock is missing.
// Reads only local files (cargo metadata --offline and node_modules).

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-._].*)?$/i;

const meta = (filter) =>
  JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--offline", "--manifest-path", join(root, "src-tauri", "Cargo.toml"), ...filter],
      { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    ),
  );

/** Packages actually built for Windows x64, reached from CoCoKey through normal and build dependencies. */
function windowsPackages() {
  const m = meta(["--filter-platform", "x86_64-pc-windows-msvc"]);
  const byId = new Map(m.packages.map((p) => [p.id, p]));
  const nodes = new Map(m.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const stack = [m.resolve.root];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const d of nodes.get(id)?.deps ?? []) {
      if (d.dep_kinds.some((k) => k.kind !== "dev")) stack.push(d.pkg);
    }
  }
  seen.delete(m.resolve.root);
  return [...seen].map((id) => byId.get(id)).sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** Every package named in Cargo.lock (also the ones only used on other systems). */
function lockPackages() {
  const lock = readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8");
  const out = [];
  for (const block of lock.split("[[package]]").slice(1)) {
    const name = /name = "([^"]+)"/.exec(block)?.[1];
    const version = /version = "([^"]+)"/.exec(block)?.[1];
    if (name && version && name !== "cocokey") out.push(`${name} ${version}`);
  }
  return out;
}

function licenseFiles(dir) {
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f))
    .sort()
    .map((f) => ({ name: f, text: readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trim() }));
}

const MIT_TERMS = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

const BSD3_TERMS = `Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`;

const LINE = "=".repeat(78);
const shared = new Map(); // hash -> { id, text }
const out = [];
out.push("CoCoKey - third-party notices", "", "CoCoKey contains the following software. Each is used under the license shown.", "");

function addPackage(title, expr, extra, files) {
  out.push(LINE, title, `License: ${expr}`, ...extra, "");
  if (files.length === 0) return false;
  for (const f of files) {
    // Long texts that many packages share word for word (the Apache License)
    // are printed once at the end and referred to here.
    if (f.text.length > 4000) {
      const h = createHash("sha256").update(f.text).digest("hex");
      if (!shared.has(h)) shared.set(h, { id: `[T${shared.size + 1}]`, text: f.text });
      out.push(`${f.name}: see ${shared.get(h).id} at the end of this file.`, "");
    } else {
      out.push(`--- ${f.name}`, f.text, "");
    }
  }
  return true;
}

// JavaScript built into the window.
const npm = [
  ["@tauri-apps/api", join(root, "node_modules", "@tauri-apps", "api")],
  ["vite (module preload helper)", join(root, "node_modules", "vite")],
];
for (const [name, dir] of npm) {
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  let files = licenseFiles(dir).filter((f) => !/\.spdx$/i.test(f.name));
  if (name.startsWith("vite")) {
    // Only Vite's own code is in CoCoKey; keep the core license part.
    files = files.map((f) => ({ ...f, text: f.text.split(/\n# Licenses of bundled dependencies/)[0].trim() }));
  }
  addPackage(`${name} ${pkg.version} (npm)`, pkg.license, pkg.repository ? [`Source: ${pkg.repository.url ?? pkg.repository}`] : [], files);
}

const missing = [];
const built = windowsPackages();
const builtKeys = new Set(built.map((p) => `${p.name} ${p.version}`));
for (const p of built) {
  const extra = [];
  if (p.repository) extra.push(`Source: ${p.repository}`);
  const files = licenseFiles(dirname(p.manifest_path));
  if (!addPackage(`${p.name} ${p.version}`, p.license ?? "see the package", extra, files)) {
    // The package carries no license file: give its authors and the terms.
    missing.push(`${p.name} ${p.version} (${p.license})`);
    const who = (p.authors ?? []).join(", ") || `the ${p.name} authors`;
    if (/\bMIT\b/.test(p.license ?? "")) out.push(`Copyright (c) ${who}`, "", MIT_TERMS, "");
    else if (/BSD-3-Clause/.test(p.license ?? "")) out.push(`Copyright (c) ${who}`, "", BSD3_TERMS, "");
    else if (/MPL-2.0/.test(p.license ?? "")) out.push(`Copyright (c) ${who}`, "This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. You can obtain one at https://mozilla.org/MPL/2.0/. The source code is available at the Source address above.", "");
    else out.push(`Copyright (c) ${who}`, `Licensed under ${p.license}.`, "");
  }
}

const notBuilt = lockPackages().filter((k) => !builtKeys.has(k));
out.push(LINE, "Listed in Cargo.lock but not built into CoCoKey for Windows:", ...notBuilt.map((k) => `  ${k}`), "");

for (const { id, text } of shared.values()) out.push(LINE, id, "", text, "");

out.push(LINE, "CoCoKey", "", readFileSync(join(root, "LICENSE"), "utf8").replace(/\r\n/g, "\n").trim(), "");

writeFileSync(join(root, "THIRD-PARTY-NOTICES.txt"), out.join("\n").replace(/\n{3,}/g, "\n\n"), "utf8");
console.log(`packages: ${built.length} built, ${notBuilt.length} not built; shared texts: ${shared.size}`);
if (missing.length) console.log(`without a license file (authors and terms written instead): ${missing.join(", ")}`);
