import { GitBranch } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AgentState } from "@/types/protocol";

interface StatusBarProps {
	className?: string;
	branch?: string;
	language?: string;
	encoding?: string;
	eol?: "LF" | "CRLF";
	agentState?: AgentState;
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
	agentState,
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
				{agentState && (
					<span
						className={`inline-flex items-center gap-1 ${
							agentState === "running"
								? "text-info"
								: agentState === "waiting"
									? "text-warning"
									: agentState === "done"
										? "text-success"
										: "text-destructive"
						}`}
					>
						<span
							className={`w-1.5 h-1.5 rounded-full ${
								agentState === "running"
									? "bg-info animate-pulse"
									: agentState === "waiting"
										? "bg-warning animate-pulse"
										: agentState === "done"
											? "bg-success"
											: "bg-destructive"
							}`}
						/>
						Agent: {agentState}
					</span>
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
