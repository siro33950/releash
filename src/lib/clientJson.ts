import {
	type DescField,
	type DescMessage,
	getOption,
	type JsonValue,
	ScalarType,
} from "@bufbuild/protobuf";
import {
	json_content,
	json_enum_name,
	json_flatten,
	json_literal,
	json_nullable,
	json_omit_empty,
	json_omit_none,
	json_required,
	json_tag,
	json_unit,
	json_untagged,
	json_wrapper,
} from "@/generated/client_options_pb";

function object(value: JsonValue): { [key: string]: JsonValue } {
	if (value === null || typeof value !== "object" || Array.isArray(value))
		throw new Error("Expected object");
	return value;
}

export function clientJson(
	schema: DescMessage,
	value: JsonValue,
	encode: boolean,
): JsonValue {
	if (getOption(schema, json_unit)) {
		if (encode && value !== null) throw new Error("Expected null");
		return encode ? {} : null;
	}
	const wrapper = getOption(schema, json_wrapper);
	if (schema.oneofs.some((oneof) => oneof.name === "variant")) {
		const tag = getOption(schema, json_tag),
			content = getOption(schema, json_content);
		for (const field of schema.fields) {
			const unit = field.message && getOption(field.message, json_unit);
			if (encode) {
				let item: JsonValue | undefined;
				if (getOption(schema, json_untagged)) item = value;
				else if (tag && object(value)[tag] === field.jsonName) {
					if (content) item = object(value)[content];
					else {
						const { [tag]: _, ...fields } = object(value);
						item = unit ? null : fields;
					}
				}
				if (item === undefined) continue;
				try {
					return { [field.jsonName]: fieldJson(field, item, true) };
				} catch (error) {
					if (!getOption(schema, json_untagged)) throw error;
				}
			} else if (field.jsonName in object(value)) {
				const item = fieldJson(field, object(value)[field.jsonName], false);
				if (getOption(schema, json_untagged)) return item;
				return content
					? { [tag]: field.jsonName, [content]: item }
					: { ...(unit ? {} : object(item)), [tag]: field.jsonName };
			}
		}
		throw new Error(`Invalid ${schema.name} variant`);
	}
	const input = encode && wrapper ? { [wrapper]: value } : object(value);
	const output: { [key: string]: JsonValue } = {};
	for (const field of schema.fields) {
		let item =
			encode && getOption(field, json_flatten)
				? Object.fromEntries(
						Object.entries(input).filter(
							([key]) => !schema.fields.some((field) => field.jsonName === key),
						),
					)
				: input[field.jsonName];
		if (item === undefined) {
			if (getOption(field, json_required))
				throw new Error(`Missing ${schema.name}.${field.jsonName}`);
			if (
				!encode &&
				getOption(field, json_nullable) &&
				!getOption(field, json_omit_none)
			)
				output[field.jsonName] = null;
			else if (!encode && field.fieldKind === "list")
				output[field.jsonName] = [];
			else if (!encode && field.fieldKind === "map")
				output[field.jsonName] = {};
			continue;
		}
		if (item === null && getOption(field, json_nullable)) {
			if (!encode) output[field.jsonName] = null;
			continue;
		}
		item = fieldJson(field, item, encode);
		if (
			!encode &&
			getOption(field, json_omit_empty) &&
			typeof item === "object" &&
			item !== null &&
			Object.keys(item).length === 0
		)
			continue;
		if (!encode && getOption(field, json_flatten))
			Object.assign(output, object(item));
		else output[field.jsonName] = item;
	}
	if (encode) {
		for (const key of Object.keys(input))
			if (
				!schema.fields.some(
					(field) => field.jsonName === key || getOption(field, json_flatten),
				)
			)
				throw new Error(`Unknown ${schema.name}.${key}`);
		return output;
	}
	return wrapper ? output[wrapper] : output;
}

function fieldJson(
	field: DescField,
	value: JsonValue,
	encode: boolean,
): JsonValue {
	const literal = getOption(field, json_literal);
	if (
		literal &&
		JSON.parse(literal) !== value &&
		!(
			typeof value === "string" &&
			typeof JSON.parse(literal) === "number" &&
			Number(value) === JSON.parse(literal)
		)
	)
		throw new Error("Invalid literal");
	const item = (value: JsonValue): JsonValue => {
		if (field.message) return clientJson(field.message, value, encode);
		if (field.enum) {
			const variant = field.enum.values.find(
				(variant) =>
					(encode ? getOption(variant, json_enum_name) : variant.name) ===
					value,
			);
			if (!variant) throw new Error("Invalid enum value");
			return encode ? variant.name : getOption(variant, json_enum_name);
		}
		if (
			field.scalar === ScalarType.STRING ||
			field.scalar === ScalarType.BOOL
		) {
			if (
				typeof value !==
				(field.scalar === ScalarType.STRING ? "string" : "boolean")
			)
				throw new Error("Invalid scalar");
			return value;
		}
		const number = !encode && typeof value === "string" ? Number(value) : value;
		if (typeof number !== "number" || !Number.isFinite(number))
			throw new Error("Invalid number");
		if (
			field.scalar !== ScalarType.DOUBLE &&
			field.scalar !== ScalarType.FLOAT &&
			!(encode ? Number.isSafeInteger(number) : Number.isInteger(number))
		)
			throw new Error("Unsafe integer");
		return number;
	};
	if (field.fieldKind === "list") {
		if (!Array.isArray(value)) throw new Error("Expected list");
		return value.map(item);
	}
	if (field.fieldKind === "map")
		return Object.fromEntries(
			Object.entries(object(value)).map(([key, value]) => [key, item(value)]),
		);
	return item(value);
}
