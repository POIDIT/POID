/**
 * Validates the conformance fixtures against spec/schema/manifest.schema.json.
 *
 * - every manifest in spec/conformance/valid/   MUST pass
 * - every manifest in spec/conformance/invalid/ MUST fail
 *
 * Exit code 0 = the schema and the fixtures agree; anything else is a bug in
 * one of them (never let them quietly diverge — CONVENTIONS.md).
 */

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv2020Module from "ajv/dist/2020.js";
import addFormatsModule from "ajv-formats";

const Ajv2020 = Ajv2020Module.default ?? Ajv2020Module;
const addFormats = addFormatsModule.default ?? addFormatsModule;

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const schemaPath = join(root, "spec", "schema", "manifest.schema.json");
const schema = JSON.parse(readFileSync(schemaPath, "utf8"));

const ajv = new Ajv2020({ allErrors: true });
addFormats(ajv);
const validate = ajv.compile(schema);

let failures = 0;

function check(dir, expectValid) {
  const dirPath = join(root, "spec", "conformance", dir);
  const files = readdirSync(dirPath).filter((f) => f.endsWith(".json"));
  if (files.length === 0) {
    console.error(`FAIL  spec/conformance/${dir} contains no fixtures`);
    failures += 1;
    return;
  }
  for (const file of files) {
    const manifest = JSON.parse(readFileSync(join(dirPath, file), "utf8"));
    const valid = validate(manifest);
    if (valid === expectValid) {
      console.log(`ok    ${dir}/${file}`);
    } else {
      failures += 1;
      console.error(`FAIL  ${dir}/${file} — expected ${expectValid ? "valid" : "invalid"}`);
      if (!valid) {
        for (const err of validate.errors ?? []) {
          console.error(`      ${err.instancePath || "/"} ${err.message}`);
        }
      }
    }
  }
}

check("valid", true);
check("invalid", false);

if (failures > 0) {
  console.error(`\n${failures} fixture(s) disagree with the schema.`);
  process.exit(1);
}
console.log("\nSchema and conformance fixtures agree.");
