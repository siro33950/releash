import { invoke } from "@tauri-apps/api/core";
import { ChevronRight } from "lucide-react";
import { useEffect, useId, useMemo, useState } from "react";
import Markdown from "react-markdown";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { rehypePluginList, remarkPluginList } from "@/lib/markdownConfig";
import { cn } from "@/lib/utils";
import type { PermissionRequest } from "@/types/session";
import { AgentEditPreviewPanel } from "./AgentEditPreviewPanel";

interface AskQuestion {
	question: string;
	header: string;
	options: { label: string; description: string }[];
	multiSelect: boolean;
}

interface PermissionPresentation {
	kind: "tool" | "exit_plan" | "ask_user_question";
	canEditInput: boolean;
	canEditContent: boolean;
	canEditMultiEditContent: boolean;
	directContentEditLabel?: string | null;
	directContent: string;
	multiEditReplacementContents: string[];
	multiEditOldStrings: string[];
	hasResolvedDetail: boolean;
	plan: string;
	allowedPrompts: { tool: string; prompt: string }[];
	questions: AskQuestion[];
}

function emptyPermissionPresentation(): PermissionPresentation {
	return {
		kind: "tool",
		canEditInput: false,
		canEditContent: false,
		canEditMultiEditContent: false,
		directContentEditLabel: null,
		directContent: "",
		multiEditReplacementContents: [],
		multiEditOldStrings: [],
		hasResolvedDetail: false,
		plan: "",
		allowedPrompts: [],
		questions: [],
	};
}

function normalizePermissionPresentation(
	presentation: Partial<PermissionPresentation> | null | undefined,
): PermissionPresentation {
	const fallback = emptyPermissionPresentation();
	if (!presentation) return fallback;
	return {
		...fallback,
		...presentation,
		directContentEditLabel: presentation.directContentEditLabel ?? null,
		directContent: presentation.directContent ?? "",
		multiEditReplacementContents: Array.isArray(
			presentation.multiEditReplacementContents,
		)
			? presentation.multiEditReplacementContents
			: [],
		multiEditOldStrings: Array.isArray(presentation.multiEditOldStrings)
			? presentation.multiEditOldStrings
			: [],
		allowedPrompts: Array.isArray(presentation.allowedPrompts)
			? presentation.allowedPrompts
			: [],
		questions: Array.isArray(presentation.questions)
			? presentation.questions
			: [],
	};
}

function InlineMarkdown({
	children,
	className,
	id,
	"data-testid": dataTestId,
}: {
	children: string;
	className?: string;
	id?: string;
	"data-testid"?: string;
}) {
	return (
		<div
			id={id}
			data-testid={dataTestId}
			className={cn(
				"markdown-preview prose prose-sm dark:prose-invert max-w-none break-words",
				className,
			)}
		>
			<Markdown
				remarkPlugins={remarkPluginList}
				rehypePlugins={rehypePluginList}
				components={{
					table: ({ children: c, ...props }) => (
						<div style={{ overflowX: "auto", maxWidth: "100%" }}>
							<table {...props}>{c}</table>
						</div>
					),
				}}
			>
				{children}
			</Markdown>
		</div>
	);
}

function PlanContent({
	plan,
	allowedPrompts,
}: {
	plan: string;
	allowedPrompts: { tool: string; prompt: string }[];
}) {
	return (
		<>
			{plan && (
				<InlineMarkdown data-testid="plan-markdown">{plan}</InlineMarkdown>
			)}
			{allowedPrompts.length > 0 && (
				<div data-testid="allowed-prompts">
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
		</>
	);
}

interface PermissionDialogProps {
	request: PermissionRequest;
	status?: "pending" | "allowed" | "denied";
	resolvedAnswers?: Record<string, string>;
	worktreePath?: string;
	onAllow: (requestId: string, updatedInput?: Record<string, unknown>) => void;
	onDeny: (requestId: string) => void;
	onAnswer?: (requestId: string, answers: Record<string, string>) => void;
}

function ResolvedDetail({
	request,
	presentation,
	resolvedAnswers,
}: {
	request: PermissionRequest;
	presentation: PermissionPresentation;
	resolvedAnswers?: Record<string, string>;
}) {
	if (presentation.kind === "exit_plan") {
		const { plan, allowedPrompts } = presentation;
		if (!plan && allowedPrompts.length === 0) return null;
		return (
			<div className="mt-1.5 space-y-1.5">
				<PlanContent plan={plan} allowedPrompts={allowedPrompts} />
			</div>
		);
	}

	if (presentation.kind === "ask_user_question") {
		const questions = presentation.questions;
		if (questions.length === 0) return null;
		return (
			<div className="mt-1.5 space-y-1">
				{questions.map((q) => (
					<div key={q.question} className="text-xs">
						<InlineMarkdown className="text-muted-foreground">
							{q.question}
						</InlineMarkdown>
						{resolvedAnswers?.[q.question] && (
							<InlineMarkdown className="font-medium">
								{resolvedAnswers[q.question]}
							</InlineMarkdown>
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
	worktreePath,
	onAllow,
	onDeny,
	onAnswer,
}: PermissionDialogProps) {
	const [answers, setAnswers] = useState<Record<string, string | string[]>>({});
	const [otherTexts, setOtherTexts] = useState<Record<string, string>>({});
	const [isExpanded, setIsExpanded] = useState(false);
	const [editedInputText, setEditedInputText] = useState(() =>
		JSON.stringify(request.input ?? {}, null, 2),
	);
	const [editedContentText, setEditedContentText] = useState("");
	const [multiEditContentTexts, setMultiEditContentTexts] = useState<string[]>(
		[],
	);
	const [editedPreviewInput, setEditedPreviewInput] = useState<Record<
		string,
		unknown
	> | null>(null);
	const [presentation, setPresentation] = useState<PermissionPresentation>(
		emptyPermissionPresentation,
	);
	const [contentEditError, setContentEditError] = useState<string | null>(null);
	const [previewEditError, setPreviewEditError] = useState<string | null>(null);
	const questionIdBase = useId();
	useEffect(() => {
		setEditedInputText(JSON.stringify(request.input ?? {}, null, 2));
		setEditedContentText("");
		setMultiEditContentTexts([]);
		setEditedPreviewInput(null);
		setContentEditError(null);
		setPreviewEditError(null);
	}, [request.input]);
	useEffect(() => {
		let canceled = false;
		setPresentation(emptyPermissionPresentation());
		void invoke<Partial<PermissionPresentation> | null>(
			"present_agent_permission_request",
			{
				toolName: request.tool_name,
				input: request.input ?? {},
			},
		)
			.then((nextPresentation) => {
				if (canceled) return;
				const normalized = normalizePermissionPresentation(nextPresentation);
				setPresentation(normalized);
				setEditedContentText(normalized.directContent);
				setMultiEditContentTexts(normalized.multiEditReplacementContents);
			})
			.catch(() => {
				if (canceled) return;
				setPresentation(emptyPermissionPresentation());
				setEditedContentText("");
				setMultiEditContentTexts([]);
			});
		return () => {
			canceled = true;
		};
	}, [request.input, request.tool_name]);
	const canEditInput = presentation.canEditInput;
	const canEditContent = presentation.canEditContent;
	const canEditMultiEditContent = presentation.canEditMultiEditContent;
	const multiEditContentCount = multiEditContentTexts.length;
	const editedInput = useMemo((): Record<string, unknown> | null => {
		if (!canEditInput) return null;
		try {
			const parsed = JSON.parse(editedInputText);
			if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
				return parsed as Record<string, unknown>;
			}
			return null;
		} catch {
			return null;
		}
	}, [canEditInput, editedInputText]);
	const allowContentEdit = async () => {
		setContentEditError(null);
		try {
			const updatedInput = await invoke<Record<string, unknown>>(
				"build_agent_edited_tool_input",
				{
					toolName: request.tool_name,
					input: editedInput ?? request.input,
					editedContent: editedContentText,
				},
			);
			onAllow(request.request_id, updatedInput);
		} catch (err) {
			setContentEditError(String(err));
		}
	};
	const allowMultiEditContentEdit = async (editIndex: number) => {
		setContentEditError(null);
		try {
			const updatedInput = await invoke<Record<string, unknown>>(
				"build_agent_edited_multi_edit_tool_input",
				{
					input: editedInput ?? request.input,
					editIndex,
					editedContent: multiEditContentTexts[editIndex] ?? "",
				},
			);
			onAllow(request.request_id, updatedInput);
		} catch (err) {
			setContentEditError(String(err));
		}
	};
	useEffect(() => {
		if (!canEditContent) return;
		let canceled = false;
		invoke<Record<string, unknown>>("build_agent_edited_tool_input", {
			toolName: request.tool_name,
			input: editedInput ?? request.input,
			editedContent: editedContentText,
		})
			.then((updatedInput) => {
				if (canceled) return;
				setEditedPreviewInput(updatedInput);
				setPreviewEditError(null);
			})
			.catch((err) => {
				if (canceled) return;
				setEditedPreviewInput(null);
				setPreviewEditError(String(err));
			});
		return () => {
			canceled = true;
		};
	}, [
		canEditContent,
		editedContentText,
		editedInput,
		request.input,
		request.tool_name,
	]);
	useEffect(() => {
		if (!canEditMultiEditContent || multiEditContentCount === 0) return;
		let canceled = false;
		invoke<Record<string, unknown>>(
			"build_agent_edited_multi_edit_tool_input_all",
			{
				input: editedInput ?? request.input,
				editedContents: multiEditContentTexts,
			},
		)
			.then((updatedInput) => {
				if (canceled) return;
				setEditedPreviewInput(updatedInput);
				setPreviewEditError(null);
			})
			.catch((err) => {
				if (canceled) return;
				setEditedPreviewInput(null);
				setPreviewEditError(String(err));
			});
		return () => {
			canceled = true;
		};
	}, [
		canEditMultiEditContent,
		editedInput,
		multiEditContentCount,
		multiEditContentTexts,
		request.input,
	]);
	const multiEditContentRows = useMemo(
		() =>
			Array.from({ length: multiEditContentCount }, (_, index) => {
				const oldString = presentation.multiEditOldStrings[index] ?? "";
				return {
					key: `${oldString}\u0000${index}`,
					oldString,
				};
			}),
		[multiEditContentCount, presentation.multiEditOldStrings],
	);
	const previewInput = editedPreviewInput ?? editedInput ?? request.input;
	if (status !== "pending") {
		const isAllowed = status === "allowed";
		let label: string;
		if (presentation.kind === "exit_plan") {
			label = isAllowed ? "Plan approved" : "Plan denied";
		} else if (presentation.kind === "ask_user_question") {
			const answerSummary = resolvedAnswers
				? Object.values(resolvedAnswers).join(", ")
				: "";
			label = answerSummary ? `Answered: ${answerSummary}` : "Answered";
		} else {
			const toolLabel =
				request.title || request.display_name || request.tool_name;
			label = `${toolLabel} — ${status}`;
		}

		const hasDetail = presentation.hasResolvedDetail;

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
					<ResolvedDetail
						request={request}
						presentation={presentation}
						resolvedAnswers={resolvedAnswers}
					/>
				)}
			</div>
		);
	}

	if (presentation.kind === "ask_user_question" && onAnswer) {
		const questions = presentation.questions;

		const OTHER_LABEL = "__other__";

		const handleSelect = (
			questionText: string,
			label: string,
			multi: boolean,
		) => {
			if (multi) {
				setAnswers((prev) => {
					const current = Array.isArray(prev[questionText])
						? (prev[questionText] as string[])
						: [];
					const next = current.includes(label)
						? current.filter((l) => l !== label)
						: [...current, label];
					return { ...prev, [questionText]: next };
				});
			} else {
				setAnswers((prev) => ({ ...prev, [questionText]: label }));
			}
		};

		const allAnswered = questions.every((q) => {
			const selected = answers[q.question];
			if (!selected) return false;
			if (Array.isArray(selected)) return selected.length > 0;
			if (selected === OTHER_LABEL) return !!otherTexts[q.question]?.trim();
			return true;
		});

		const handleSubmit = () => {
			if (!allAnswered) return;
			const resolved: Record<string, string> = {};
			for (const q of questions) {
				const selected = answers[q.question];
				if (Array.isArray(selected)) {
					resolved[q.question] = selected.join(", ");
				} else {
					resolved[q.question] =
						selected === OTHER_LABEL ? otherTexts[q.question].trim() : selected;
				}
			}
			onAnswer(request.request_id, resolved);
		};

		return (
			<div
				data-testid="permission-dialog"
				className="mx-3 my-1.5 rounded-md border border-border bg-muted/50 p-3 overflow-hidden"
			>
				{questions.map((q, qIndex) => {
					const questionId = `${questionIdBase}-q-${qIndex}`;
					return (
						<div key={q.question} className="mb-2">
							<InlineMarkdown className="text-xs text-muted-foreground mb-0.5">
								{q.header}
							</InlineMarkdown>
							<InlineMarkdown
								id={questionId}
								className="text-sm font-medium mb-1.5"
							>
								{q.question}
							</InlineMarkdown>
							{q.multiSelect ? (
								<fieldset
									className="space-y-2 border-0 p-0 m-0"
									aria-labelledby={questionId}
								>
									{q.options.map((opt) => {
										const isChecked =
											Array.isArray(answers[q.question]) &&
											(answers[q.question] as string[]).includes(opt.label);
										return (
											// biome-ignore lint/a11y/noLabelWithoutControl: Radix Checkbox renders an internal button element
											<label
												key={opt.label}
												className="flex items-start gap-2.5 cursor-pointer rounded-md border border-border px-3 py-2 hover:bg-accent/50"
											>
												<Checkbox
													checked={isChecked}
													onCheckedChange={() =>
														handleSelect(q.question, opt.label, true)
													}
													className="mt-0.5"
												/>
												<div className="flex flex-col flex-1 min-w-0">
													<span className="text-sm font-medium">
														{opt.label}
													</span>
													{opt.description && (
														<InlineMarkdown className="text-xs text-muted-foreground">
															{opt.description}
														</InlineMarkdown>
													)}
												</div>
											</label>
										);
									})}
								</fieldset>
							) : (
								<RadioGroup
									value={
										typeof answers[q.question] === "string"
											? (answers[q.question] as string)
											: undefined
									}
									onValueChange={(value) =>
										handleSelect(q.question, value, false)
									}
									className="space-y-2"
									aria-labelledby={questionId}
								>
									{q.options.map((opt) => (
										// biome-ignore lint/a11y/noLabelWithoutControl: Radix RadioGroupItem renders an internal button element
										<label
											key={opt.label}
											className="flex items-start gap-2.5 cursor-pointer rounded-md border border-border px-3 py-2 hover:bg-accent/50"
										>
											<RadioGroupItem value={opt.label} className="mt-0.5" />
											<div className="flex flex-col flex-1 min-w-0">
												<span className="text-sm font-medium">{opt.label}</span>
												{opt.description && (
													<InlineMarkdown className="text-xs text-muted-foreground">
														{opt.description}
													</InlineMarkdown>
												)}
											</div>
										</label>
									))}
									{/* biome-ignore lint/a11y/noLabelWithoutControl: Radix RadioGroupItem renders an internal button element */}
									<label className="flex items-start gap-2.5 cursor-pointer rounded-md border border-border px-3 py-2 hover:bg-accent/50">
										<RadioGroupItem value={OTHER_LABEL} className="mt-0.5" />
										<div className="flex flex-col flex-1 min-w-0">
											<span className="text-sm font-medium">Other</span>
											{answers[q.question] === OTHER_LABEL && (
												<Input
													type="text"
													aria-label={`Other input for ${q.question}`}
													value={otherTexts[q.question] ?? ""}
													onClick={(e) => e.stopPropagation()}
													onChange={(e) =>
														setOtherTexts((prev) => ({
															...prev,
															[q.question]: e.target.value,
														}))
													}
													className="mt-1 h-auto text-sm"
													placeholder="Type your answer..."
												/>
											)}
										</div>
									</label>
								</RadioGroup>
							)}
						</div>
					);
				})}
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

	if (presentation.kind === "exit_plan") {
		const { plan, allowedPrompts } = presentation;

		return (
			<div
				data-testid="permission-dialog"
				className="mx-3 my-1.5 rounded-md border border-border bg-muted/50 p-3 overflow-hidden"
			>
				<p className="text-sm font-medium mb-2">Plan Review</p>
				<div className="space-y-2 mb-2">
					<PlanContent plan={plan} allowedPrompts={allowedPrompts} />
				</div>
				<AllowDenyButtons
					requestId={request.request_id}
					onAllow={onAllow}
					onDeny={onDeny}
				/>
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
				<>
					{presentation.canEditInput && (
						<div className="mb-2">
							<AgentEditPreviewPanel
								worktreePath={worktreePath}
								toolName={request.tool_name}
								input={previewInput}
							/>
							{previewEditError && (
								<p className="mt-1 text-xs text-destructive">
									Edited preview unavailable: {previewEditError}
								</p>
							)}
						</div>
					)}
					<pre
						data-testid="permission-input"
						className="text-xs bg-muted/50 rounded p-2 mb-2 max-h-32 overflow-y-auto whitespace-pre-wrap break-all"
					>
						{JSON.stringify(request.input, null, 2)}
					</pre>
					{canEditInput && (
						<>
							{canEditContent && (
								<div className="mb-2">
									<div className="mb-1 text-xs font-medium text-muted-foreground">
										{presentation.directContentEditLabel}
									</div>
									<textarea
										aria-label={
											presentation.directContentEditLabel ?? undefined
										}
										value={editedContentText}
										onChange={(event) =>
											setEditedContentText(event.target.value)
										}
										className="min-h-24 w-full resize-y rounded border bg-background p-2 font-mono text-xs outline-none focus:ring-1 focus:ring-ring"
									/>
									{contentEditError && (
										<p className="mt-1 text-xs text-destructive">
											{contentEditError}
										</p>
									)}
									<Button
										size="xs"
										variant="secondary"
										className="mt-1"
										onClick={() => void allowContentEdit()}
									>
										Allow content edit
									</Button>
								</div>
							)}
							{canEditMultiEditContent && multiEditContentCount > 0 && (
								<div className="mb-2 space-y-2">
									<div className="text-xs font-medium text-muted-foreground">
										Edit replacement content
									</div>
									{multiEditContentTexts.map((content, index) => {
										const row = multiEditContentRows[index];
										return (
											<div
												key={row.key}
												className="rounded border border-border bg-background/60 p-2"
											>
												<div className="mb-1 flex items-center justify-between gap-2 text-xs">
													<span className="font-medium">Edit {index + 1}</span>
													{row.oldString && (
														<span className="min-w-0 truncate text-muted-foreground">
															Replace: {row.oldString}
														</span>
													)}
												</div>
												<textarea
													aria-label={`Edit replacement content ${index + 1}`}
													value={content}
													onChange={(event) => {
														const next = [...multiEditContentTexts];
														next[index] = event.target.value;
														setMultiEditContentTexts(next);
													}}
													className="min-h-20 w-full resize-y rounded border bg-background p-2 font-mono text-xs outline-none focus:ring-1 focus:ring-ring"
												/>
												<Button
													size="xs"
													variant="secondary"
													className="mt-1"
													onClick={() => void allowMultiEditContentEdit(index)}
												>
													Allow edit {index + 1}
												</Button>
											</div>
										);
									})}
									{contentEditError && (
										<p className="text-xs text-destructive">
											{contentEditError}
										</p>
									)}
								</div>
							)}
							<textarea
								aria-label="Edit permission input JSON"
								value={editedInputText}
								onChange={(event) => setEditedInputText(event.target.value)}
								className="mb-2 min-h-24 w-full resize-y rounded border bg-background p-2 font-mono text-xs outline-none focus:ring-1 focus:ring-ring"
							/>
						</>
					)}
				</>
			)}
			<AllowDenyButtons
				requestId={request.request_id}
				onAllow={onAllow}
				onDeny={onDeny}
				editedInput={editedInput ?? undefined}
				showEditedAllow={canEditInput}
			/>
		</div>
	);
}

function AllowDenyButtons({
	requestId,
	onAllow,
	onDeny,
	editedInput,
	showEditedAllow = false,
}: {
	requestId: string;
	onAllow: (requestId: string, updatedInput?: Record<string, unknown>) => void;
	onDeny: (requestId: string) => void;
	editedInput?: Record<string, unknown>;
	showEditedAllow?: boolean;
}) {
	return (
		<div className="flex gap-2">
			<Button size="xs" onClick={() => onAllow(requestId)}>
				Allow
			</Button>
			{showEditedAllow && (
				<Button
					size="xs"
					variant="secondary"
					onClick={() => editedInput && onAllow(requestId, editedInput)}
					disabled={!editedInput}
				>
					Allow edited
				</Button>
			)}
			<Button size="xs" variant="outline" onClick={() => onDeny(requestId)}>
				Deny
			</Button>
		</div>
	);
}
