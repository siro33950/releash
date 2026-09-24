import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createFileRegistry, fromBinary, getOption, ScalarType } from "@bufbuild/protobuf";
import { FileDescriptorSetSchema } from "@bufbuild/protobuf/wkt";

const temporary = mkdtempSync(join(tmpdir(), "releash-protocol-"));
try {
  const descriptor = join(temporary, "client.bin");
  execFileSync("protoc", ["--proto_path=proto", "--include_imports", `--descriptor_set_out=${descriptor}`, "proto/client.proto"]);
  const registry = createFileRegistry(fromBinary(FileDescriptorSetSchema, readFileSync(descriptor)));
  const option = (desc, name) => getOption(desc, registry.getExtension(`releash.client.v1.${name}`));
  const types = new Map();
  const fieldType = (field, input = false) => {
    let type = field.message ? messageType(field.message, input) : field.enum ? field.enum.values.filter(value => option(value, "json_enum_name")).map(value => JSON.stringify(option(value, "json_enum_name"))).join(" | ") : field.scalar === ScalarType.BOOL ? "boolean" : field.scalar === ScalarType.STRING ? "string" : "number";
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
  const terminalArgs = messageType(registry.getMessage("releash.client.v1.AttachTerminalSurfaceRequest"), true);
  const mappings = [["ClientCommandArgs", "CommandRequest"], ["ClientCommandResults", "CommandResult"], ["ClientPushPayloads", "Push"]].map(([name, proto]) => {
    const fields = registry.getMessage(`releash.client.v1.${proto}`).fields.filter(field => field.message && field.oneof);
    if (name === "ClientCommandResults") return `export interface ClientCommands {\n${fields.map(field => `${JSON.stringify(field.name)}(args: ClientCommandArgs[${JSON.stringify(field.name)}]): Promise<${option(field.message, "json_unit") ? "void" : messageType(field.message)}>;`).join("\n")}\n}\nexport type ClientCommandResults = { [K in keyof ClientCommands]: Awaited<ReturnType<ClientCommands[K]>> };`;
    return `export interface ${name} {\n${name === "ClientCommandArgs" ? `"attach_terminal_surface": ${terminalArgs};\n` : ""}${fields.map(field => `${JSON.stringify(proto === "Push" ? field.name.replaceAll("_", "-") : field.name)}: ${name === "ClientCommandResults" && option(field.message, "json_unit") ? "void" : messageType(field.message, name === "ClientCommandArgs")};`).join("\n")}\n}`;
  });
  const service = registry.getService("releash.client.v1.ClientService");
  const commands = registry.getMessage("releash.client.v1.CommandRequest").fields.filter(field => field.message && field.oneof);
  const invocations = commands.map(field => {
    const method = service.methods.find(method => method.input.typeName === field.message.typeName && method.methodKind === "unary");
    if (!method) throw new Error(`Missing RPC for ${field.name}`);
    return `${JSON.stringify(field.name)}: async (client: Client<typeof ClientService>, args: ClientCommandArgs[${JSON.stringify(field.name)}]) => { const result = decode(${method.output.name}Schema, await client.${method.localName}(fromJson(${method.input.name}Schema, clientJson(${method.input.name}Schema, JSON.parse(JSON.stringify(args ?? {})), true)))); return result; }`;
  });
  const schemas = new Set(service.methods.filter(method => method.methodKind === "unary" && commands.some(field => field.message?.typeName === method.input.typeName)).flatMap(method => [method.input.name, method.output.name]));
  writeFileSync("src/generated/client_commands.ts", `// Generated from proto/client.proto. Run pnpm generate:protocol.\nimport { type DescMessage, type Message, fromJson, toJson } from "@bufbuild/protobuf";\nimport { type Client, ConnectError } from "@connectrpc/connect";\nimport { ${[...schemas].map(name => name + "Schema").join(", ")}, ClientService, CommandErrorSchema } from "./client_pb";\nimport type { ClientCommandArgs, ClientCommandResults } from "./client_types";\nimport { clientJson } from "@/lib/clientJson";\nimport { getClient } from "@/lib/client";\nconst decode = (schema: DescMessage, message: Message) => clientJson(schema, toJson(schema, message), false);\nconst commands = {\n${invocations.join(",\n")}\n};\nexport type ClientCommand = keyof typeof commands;\nexport type EmptyClientCommand = { [K in ClientCommand]: ClientCommandArgs[K] extends Record<string, never> ? K : never }[ClientCommand];\nexport async function invokeClient<K extends ClientCommand>(command: K, args?: ClientCommandArgs[K]): Promise<ClientCommandResults[K]> {\nconst client = await getClient();\ntry { return await (commands[command] as (client: Client<typeof ClientService>, args: ClientCommandArgs[K]) => Promise<unknown>)(client, args ?? {} as ClientCommandArgs[K]) as ClientCommandResults[K]; } catch (error) {\nif (error instanceof ConnectError) { const detail = error.findDetails(CommandErrorSchema)[0]; if (detail) throw decode(CommandErrorSchema, detail); }\nthrow error;\n}\n}\n`);
  writeFileSync("src/generated/client_types.ts", `// Generated from proto/client.proto. Run pnpm generate:protocol.\n\n${[...types].map(([name, type]) => `export type ${name} = ${type};`).join("\n\n")}\n\n${mappings.join("\n\n")}\n`);
} finally { rmSync(temporary, { recursive: true, force: true }); }
