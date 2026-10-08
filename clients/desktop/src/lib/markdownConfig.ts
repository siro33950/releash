import type { Options } from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

const sanitizeSchema = {
	...defaultSchema,
	attributes: {
		...defaultSchema.attributes,
		code: [...(defaultSchema.attributes?.code ?? []), "className"],
	},
};

export const rehypePluginList = [
	rehypeRaw,
	[rehypeSanitize, sanitizeSchema],
	rehypeHighlight,
] as Options["rehypePlugins"];

export const remarkPluginList = [remarkGfm, remarkBreaks];
