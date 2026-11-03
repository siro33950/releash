import { readFileSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

export function manifest(version, url, signature, date = new Date()) {
  const platform = { signature, url };
  return {
    version,
    notes: "",
    pub_date: date.toISOString(),
    platforms: Object.fromEntries(["darwin-aarch64", "darwin-x86_64", "darwin-universal",
      "darwin-aarch64-app", "darwin-x86_64-app", "darwin-universal-app"].map(key => [key, platform])),
  };
}

if (realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [version, url, signaturePath] = process.argv.slice(2);
  process.stdout.write(JSON.stringify(manifest(version, url, readFileSync(signaturePath, "utf8")), null, 2));
}
