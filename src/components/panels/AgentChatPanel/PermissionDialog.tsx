import { ChevronRight } from "lucide-react";
import { useMemo, useState } from "react";
import Markdown from "react-markdown";
import { Button } from "@/components/ui/button";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import { cn } from "@/lib/utils";
import type { PermissionRequest } from "@/types/session";

interface AskQuestion {
	question: string;
	header: string;
	options: { label: string; description: string }[];
	multiSelect: boolean;
}

interface PermissionDialogProps {
	request: PermissionRequest;
	status?: "pending" | "allowed" | "denied";
	resolvedAnswers?: Record<string, string>;
	onAllow: (requestId: string) => void;
	onDeny: (requestId: string) => void;
	onAnswer?: (requestId: string, answers: Record<string, string>) => void;
}

function hasResolvedDetail(request: PermissionRequest): boolean {
	if (request.tool_name === "ExitPlanMode") {
		const input = request.input as {
			plan?: string;
			allowedPrompts?: { tool: string; prompt: string }[];
		};
		return !!(
			input.plan ||
			(input.allowedPrompts && input.allowedPrompts.length > 0)
		);
	}
	if (request.tool_name === "AskUserQuestion") {
		const questions =
			(request.input as { questions?: AskQuestion[] }).questions ?? [];
		return questions.length > 0;
	}
	return !!(request.input && Object.keys(request.input).length > 0);
}

function ResolvedDetail({
	request,
	resolvedAnswers,
}: {
	request: PermissionRequest;
	resolvedAnswers?: Record<string, string>;
}) {
	const remarkPlugins = useMemo(() => remarkPluginList, []);

	if (request.tool_name === "ExitPlanMode") {
		const input = request.input as {
			plan?: string;
			allowedPrompts?: { tool: string; prompt: string }[];
		};
		const plan = input.plan ?? "";
		const allowedPrompts = input.allowedPrompts;
		if (!plan && (!allowedPrompts || allowedPrompts.length === 0)) return null;
		return (
			<div className="mt-1.5 space-y-1.5">
				{plan && (
					<div className="markdown-preview prose prose-sm dark:prose-invert max-w-none break-words">
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePluginList}
						>
							{plan}
						</Markdown>
					</div>
				)}
				{allowedPrompts && allowedPrompts.length > 0 && (
					<div>
						<p className="text-xs font-medium text-muted-foreground mb-0.5">
							Permissions:
						</p>
						<ul className="text-xs text-muted-foreground list-disc list-inside">
							{allowedPrompts.map((p) => (
								<li key={`${p.tool}:${p.prompt}`}>
									{p.tool}: {p.prompt}
								</li>
							))}
						</ul>
					</div>
				)}
			</div>
		);
	}

	if (request.tool_name === "AskUserQuestion") {
		const questions: AskQuestion[] =
			(request.input as { questions?: AskQuestion[] }).questions ?? [];
		if (questions.length === 0) return null;
		return (
			<div className="mt-1.5 space-y-1">
				{questions.map((q) => (
					<div key={q.question} className="text-xs">
						<p className="text-muted-foreground">{q.question}</p>
						{resolvedAnswers?.[q.question] && (
							<p className="font-medium">{resolvedAnswers[q.question]}</p>
						)}
					</div>
				))}
			</div>
		);
	}

	// Generic tool: show input JSON
	if (request.input && Object.keys(request.input).length > 0) {
		return (
			<pre className="mt-1.5 text-[11px] text-muted-foreground/70 whitespace-pre-wrap break-words overflow-hidden max-h-48 overflow-y-auto">
				{JSON.stringify(request.input, null, 2)}
			</pre>
		);
	}

	return null;
}

export function PermissionDialog({
	request,
	status = "pending",
	resolvedAnswers,
	onAllow,
	onDeny,
	onAnswer,
}: PermissionDialogProps) {
	const [answers, setAnswers] = useState<Record<string, string>>({});
	const [otherTexts, setOtherTexts] = useState<Record<string, string>>({});
	const [isExpanded, setIsExpanded] = useState(false);
	const remarkPlugins = useMemo(() => remarkPluginList, []);

	if (status !== "pending") {
		const isAllowed = status === "allowed";
		let label: string;
		if (request.tool_name === "ExitPlanMode") {
			label = isAllowed ? "Plan approved" : "Plan denied";
		} else if (request.tool_name === "AskUserQuestion") {
			const answerSummary = resolvedAnswers
				? Object.values(resolvedAnswers).join(", ")
				: "";
			label = answerSummary ? `Answered: ${answerSummary}` : "Answered";
		} else {
			const toolLabel =
				request.title || request.display_name || request.tool_name;
			label = `${toolLabel} — ${status}`;
		}

		const hasDetail = hasResolvedDetail(request);

		return (
			<div
				data-testid="permission-resolved"
				className={cn(
					"mx-3 my-1 rounded-md px-3 py-1.5 text-xs",
					isAllowed
						? "border border-green-500/20 bg-green-500/5 text-green-700 dark:text-green-400"
						: "border border-red-500/20 bg-red-500/5 text-red-700 dark:text-red-400",
				)}
			>
				<button
					type="button"
					className="flex items-center gap-1 w-full text-left"
					onClick={() => hasDetail && setIsExpanded(!isExpanded)}
					disabled={!hasDetail}
				>
					{hasDetail && (
						<ChevronRight
							className={cn(
								"size-3 shrink-0 transition-transform",
								isExpanded && "rotate-90",
							)}
						/>
					)}
					<span>
						{isAllowed ? "✓" : "✗"} {label}
					</span>
				</button>
				{isExpanded && (
					<ResolvedDetail request={request} resolvedAnswers={resolvedAnswers} />
				)}
			</div>
		);
	}

	if (request.tool_name === "AskUserQuestion" && onAnswer) {
		const questions: AskQuestion[] =
			(request.input as { questions?: AskQuestion[] }).questions ?? [];

		const OTHER_LABEL = "__other__";

		const handleSelect = (questionText: string, label: string) => {
			setAnswers((prev) => ({ ...prev, [questionText]: label }));
		};

		const allAnswered = questions.every((q) => {
			const selected = answers[q.question];
			if (!selected) return false;
			if (selected === OTHER_LABEL) return !!otherTexts[q.question]?.trim();
			return true;
		});

		const handleSubmit = () => {
			if (!allAnswered) return;
			const resolved: Record<string, string> = {};
			for (const q of questions) {
				const selected = answers[q.question];
				resolved[q.question] =
					selected === OTHER_LABEL ? otherTexts[q.question].trim() : selected;
			}
			onAnswer(request.request_id, resolved);
		};

		return (
			<div
				data-testid="permission-dialog"
				className="mx-3 my-1.5 rounded-md border border-border bg-muted/50 p-3"
			>
				{questions.map((q) => (
					<div key={q.question} className="mb-2">
						<p className="text-xs text-muted-foreground mb-0.5">{q.header}</p>
						<p className="text-sm font-medium mb-1.5">{q.question}</p>
						<div className="flex flex-wrap gap-1.5">
							{q.options.map((opt) => (
								<Button
									key={opt.label}
									size="xs"
									variant={
										answers[q.question] === opt.label ? "default" : "outline"
									}
									onClick={() => handleSelect(q.question, opt.label)}
									title={opt.description}
									className={cn(
										answers[q.question] === opt.label && "pointer-events-none",
									)}
								>
									{opt.label}
								</Button>
							))}
							<Button
								size="xs"
								variant={
									answers[q.question] === OTHER_LABEL ? "default" : "outline"
								}
								onClick={() => handleSelect(q.question, OTHER_LABEL)}
								className={cn(
									answers[q.question] === OTHER_LABEL && "pointer-events-none",
								)}
							>
								Other
							</Button>
						</div>
						{answers[q.question] === OTHER_LABEL && (
							<input
								type="text"
								aria-label={`Other input for ${q.question}`}
								value={otherTexts[q.question] ?? ""}
								onChange={(e) =>
									setOtherTexts((prev) => ({
										...prev,
										[q.question]: e.target.value,
									}))
								}
								className="mt-1.5 w-full rounded-md border border-border bg-background px-2 py-1 text-sm outline-none focus:ring-1 focus:ring-ring"
								placeholder="Type your answer..."
							/>
						)}
					</div>
				))}
				<Button
					size="xs"
					onClick={handleSubmit}
					disabled={!allAnswered}
					className="mt-1"
				>
					Submit
				</Button>
			</div>
		);
	}

	if (request.tool_name === "ExitPlanMode") {
		const input = request.input as {
			plan?: string;
			planFilePath?: string;
			allowedPrompts?: { tool: string; prompt: string }[];
		};
		const plan = input.plan ?? "";
		const allowedPrompts = input.allowedPrompts;

		return (
			<div
				data-testid="permission-dialog"
				className="mx-3 my-1.5 rounded-md border border-border bg-muted/50 p-3"
			>
				<p className="text-sm font-medium mb-2">Plan Review</p>
				{plan && (
					<div
						data-testid="plan-markdown"
						className="markdown-preview prose prose-sm dark:prose-invert max-w-none break-words mb-2"
					>
						<Markdown
							remarkPlugins={remarkPlugins}
							rehypePlugins={rehypePluginList}
						>
							{plan}
						</Markdown>
					</div>
				)}
				{allowedPrompts && allowedPrompts.length > 0 && (
					<div data-testid="allowed-prompts" className="mb-2">
						<p className="text-xs font-medium text-muted-foreground mb-1">
							Permissions:
						</p>
						<ul className="text-xs text-muted-foreground list-disc list-inside">
							{allowedPrompts.map((p) => (
								<li key={`${p.tool}:${p.prompt}`}>
									{p.tool}: {p.prompt}
								</li>
							))}
						</ul>
					</div>
				)}
				<div className="flex gap-2">
					<Button size="xs" onClick={() => onAllow(request.request_id)}>
						Allow
					</Button>
					<Button
						size="xs"
						variant="outline"
						onClick={() => onDeny(request.request_id)}
					>
						Deny
					</Button>
				</div>
			</div>
		);
	}

	const toolLabel = request.title || request.display_name || request.tool_name;

	return (
		<div
			data-testid="permission-dialog"
			className="mx-3 my-1.5 rounded-md border border-border bg-muted/50 p-3"
		>
			<p className="text-sm font-medium mb-1">
				Permission required: {toolLabel}
			</p>
			{request.description && (
				<p className="text-xs text-muted-foreground mb-2">
					{request.description}
				</p>
			)}
			{request.input && Object.keys(request.input).length > 0 && (
				<pre
					data-testid="permission-input"
					className="text-xs bg-muted/50 rounded p-2 mb-2 max-h-32 overflow-y-auto whitespace-pre-wrap break-all"
				>
					{JSON.stringify(request.input, null, 2)}
				</pre>
			)}
			<div className="flex gap-2">
				<Button size="xs" onClick={() => onAllow(request.request_id)}>
					Allow
				</Button>
				<Button
					size="xs"
					variant="outline"
					onClick={() => onDeny(request.request_id)}
				>
					Deny
				</Button>
			</div>
		</div>
	);
}
