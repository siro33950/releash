import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createFileRegistry, fromBinary, getOption, ScalarType } from "@bufbuild/protobuf";
import { FileDescriptorSetSchema } from "@bufbuild/protobuf/wkt";

const temporary = mkdtempSync(join(tmpdir(), "releash-protocol-"));
try {
  const transport = join(temporary, "client-transport");
  execFileSync("rustc", ["--edition=2021", "scripts/generate-client-transport.rs", "-o", transport]);
  writeFileSync("src/generated/client_transport.ts", execFileSync(transport));
  const descriptor = join(temporary, "client.bin");
  execFileSync("protoc", ["--proto_path=proto", "--include_imports", `--descriptor_set_out=${descriptor}`, "proto/client.proto"]);
  const registry = createFileRegistry(fromBinary(FileDescriptorSetSchema, readFileSync(descriptor)));
  const option = (desc, name) => getOption(desc, registry.getExtension(`releash.client.v1.${name}`));
  const types = new Map();
  const fieldType = (field, input = false) => {
    let type = field.message ? messageType(field.message, input) : field.enum ? field.enum.values.map(value => JSON.stringify(option(value, "json_enum_name"))).join(" | ") : field.scalar === ScalarType.BOOL ? "boolean" : field.scalar === ScalarType.STRING ? "string" : "number";
    if (option(field, "json_literal")) type = option(field, "json_literal");
    if (field.fieldKind === "list") type = `Array<${type}>`;
    if (field.fieldKind === "map") type = `{ [key: string]: ${type} }`;
    if (option(field, "json_nullable") && (input || !option(field, "json_omit_none"))) type += " | null";
    return type;
  };
  function messageType(desc, input = false) {
    if (option(desc, "json_unit")) return "null";
    const name = (input ? "Input" : "") + desc.name;
    if (types.has(name)) return name;
    types.set(name, "");
    let shape;
    if (option(desc, "json_wrapper")) shape = fieldType(desc.fields[0], input);
    else if (desc.oneofs.some(oneof => oneof.name === "variant")) {
      const tag = option(desc, "json_tag"), content = option(desc, "json_content");
      shape = desc.fields.map(field => {
        const item = fieldType(field, input), variant = JSON.stringify(field.jsonName);
        if (option(desc, "json_untagged")) return item;
        return content ? `{ ${JSON.stringify(tag)}: ${variant}; ${JSON.stringify(content)}: ${item} }` : `{ ${JSON.stringify(tag)}: ${variant} }${item === "null" ? "" : ` & ${item}`}`;
      }).join(" | ");
    } else shape = `{\n${desc.fields.filter(field => !option(field, "json_flatten")).map(field => `${JSON.stringify(field.jsonName)}${option(field, "json_required") || (!input && !option(field, "json_omit_none") && (option(field, "json_nullable") || (option(field, "json_default") && !option(field, "json_omit_empty")))) ? "" : "?"}: ${fieldType(field, input)};`).join("\n")}\n}`;
    for (const field of desc.fields.filter(field => option(field, "json_flatten"))) shape += ` & ${fieldType(field, input)}`;
    if (shape === "{\n\n}") shape = "Record<string, never>";
    types.set(name, shape);
    return name;
  }
  const mappings = [["ClientCommandArgs", "CommandRequest"], ["ClientCommandResults", "CommandResult"], ["ClientPushPayloads", "Push"]].map(([name, proto]) => {
    const fields = registry.getMessage(`releash.client.v1.${proto}`).fields.filter(field => field.message && field.oneof);
    if (name === "ClientCommandResults") return `export interface ClientCommands {\n${fields.map(field => `${JSON.stringify(field.name)}(args: ClientCommandArgs[${JSON.stringify(field.name)}]): Promise<${option(field.message, "json_unit") ? "void" : messageType(field.message)}>;`).join("\n")}\n}\nexport type ClientCommandResults = { [K in keyof ClientCommands]: Awaited<ReturnType<ClientCommands[K]>> };`;
    return `export interface ${name} {\n${fields.map(field => `${JSON.stringify(proto === "Push" ? field.name.replaceAll("_", "-") : field.name)}: ${name === "ClientCommandResults" && option(field.message, "json_unit") ? "void" : messageType(field.message, name === "ClientCommandArgs")};`).join("\n")}\n}`;
  });
  writeFileSync("src/generated/client_types.ts", `// Generated from proto/client.proto. Run pnpm generate:protocol.\n\n${[...types].map(([name, type]) => `export type ${name} = ${type};`).join("\n\n")}\n\n${mappings.join("\n\n")}\n`);
} finally { rmSync(temporary, { recursive: true, force: true }); }
