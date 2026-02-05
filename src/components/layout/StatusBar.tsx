import { GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";

interface StatusBarProps {
	className?: string;
	branch?: string;
	language?: string;
	encoding?: string;
	eol?: "LF" | "CRLF";
}

const languageDisplayNames: Record<string, string> = {
	typescript: "TypeScript",
	javascript: "JavaScript",
	json: "JSON",
	markdown: "Markdown",
	css: "CSS",
	scss: "SCSS",
	less: "Less",
	html: "HTML",
	xml: "XML",
	yaml: "YAML",
	toml: "TOML",
	rust: "Rust",
	go: "Go",
	python: "Python",
	ruby: "Ruby",
	java: "Java",
	kotlin: "Kotlin",
	swift: "Swift",
	c: "C",
	cpp: "C++",
	csharp: "C#",
	shell: "Shell",
	sql: "SQL",
	graphql: "GraphQL",
	vue: "Vue",
	svelte: "Svelte",
	plaintext: "Plain Text",
};

function getLanguageDisplayName(languageId: string): string {
	return languageDisplayNames[languageId] ?? languageId;
}

export function StatusBar({
	className,
	branch,
	language,
	encoding,
	eol,
}: StatusBarProps) {
	const hasFileInfo = language != null;

	return (
		<div
			className={cn(
				"flex items-center justify-between h-6 px-3 select-none",
				"bg-primary text-primary-foreground text-xs",
				className,
			)}
		>
			<div className="flex items-center gap-1">
				{branch && (
					<>
						<GitBranch className="w-3.5 h-3.5" />
						<span>{branch}</span>
					</>
				)}
			</div>
			{hasFileInfo && (
				<div className="flex items-center gap-4">
					{encoding && <span>{encoding}</span>}
					{eol && <span>{eol}</span>}
					<span>{getLanguageDisplayName(language)}</span>
				</div>
			)}
		</div>
	);
}
