import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { appendFileSync } from "node:fs";

for (const [name, reference] of Object.entries(process.env)) {
  if (!reference.startsWith("op://")) continue;
  const value = execFileSync("op", ["read", "--no-newline", reference], { encoding: "utf8" });
  const masked = value.replaceAll("%", "%25").replaceAll("\r", "%0D").replaceAll("\n", "%0A");
  console.log(`::add-mask::${masked}`);
  const delimiter = randomUUID();
  appendFileSync(process.env.GITHUB_ENV, `${name}<<${delimiter}\n${value}\n${delimiter}\n`);
}
