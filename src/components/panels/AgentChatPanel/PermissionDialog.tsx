import { invoke } from "@tauri-apps/api/core";
import {
	Check,
	ChevronRight,
	ClipboardCheck,
	HelpCircle,
	Shield,
	X,
} from "lucide-react";
import type React from "react";
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
import { UserMessage } from "./StreamMessage";

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

function PermissionKindIcon({
	kind,
}: {
	kind: PermissionPresentation["kind"];
}) {
	const Icon =
		kind === "exit_plan"
			? ClipboardCheck
			: kind === "ask_user_question"
				? HelpCircle
				: Shield;
	return <Icon className="size-3.5 shrink-0" />;
}

function PermissionShell({
	children,
	"data-testid": dataTestId = "permission-dialog",
}: {
	children: React.ReactNode;
	"data-testid"?: string;
}) {
	return (
		<div
			data-testid={dataTestId}
			className="mx-3 my-2 overflow-hidden rounded border border-border bg-background px-2 py-2 text-xs"
		>
			{children}
		</div>
	);
}

interface PermissionDialogProps {
	request: PermissionRequest;
	status?: "pending" | "allowed" | "denied";
	resolvedAnswers?: Record<string, string>;
	worktreePath?: string;
	onOpenDiffFile?: (filePath: string) => void;
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
			<div className="mt-1.5 space-y-2">
				{questions.map((q) => {
					const selectedRaw = resolvedAnswers?.[q.question] ?? "";
					const selectedLabels = new Set(
						selectedRaw
							? q.multiSelect
								? selectedRaw.split(", ")
								: [selectedRaw]
							: [],
					);
					return (
						<div key={q.question} className="text-xs">
							<InlineMarkdown className="text-muted-foreground">
								{q.question}
							</InlineMarkdown>
							{q.options.length > 0 && (
								<div className="mt-1 space-y-0.5">
									{q.options.map((opt) => {
										const isSelected = selectedLabels.has(opt.label);
										return (
											<div
												key={opt.label}
												data-testid="resolved-option"
												data-selected={isSelected}
												className={cn(
													"flex items-start gap-1.5 rounded px-1.5 py-0.5",
													isSelected
														? "bg-foreground/5 text-foreground"
														: "text-muted-foreground",
												)}
											>
												{isSelected ? (
													<Check className="mt-0.5 size-3 shrink-0" />
												) : (
													<span className="mt-0.5 size-3 shrink-0" />
												)}
												<span className="min-w-0">
													<span className={isSelected ? "font-medium" : ""}>
														{opt.label}
													</span>
													{opt.description && (
														<span className="text-muted-foreground">
															{" — "}
															{opt.description}
														</span>
													)}
												</span>
											</div>
										);
									})}
								</div>
							)}
						</div>
					);
				})}
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
	onOpenDiffFile,
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
	const [presentationReady, setPresentationReady] = useState(false);
	const [contentEditError, setContentEditError] = useState<string | null>(null);
	const [previewEditError, setPreviewEditError] = useState<string | null>(null);
	const questionIdBase = useId();
	// request.input はバックエンドのスナップショット更新ごとに新しい参照になるため、
	// 参照を直接 effect 依存にすると内容が同じでも毎回 effect が再実行され、
	// presentation 取得のリセット（シマー）が繰り返されてチラつく。内容ベースの
	// 安定キーに変換して再取得を抑止する。
	const inputKey = useMemo(
		() => JSON.stringify(request.input ?? {}),
		[request.input],
	);
	useEffect(() => {
		setEditedInputText(JSON.stringify(request.input ?? {}, null, 2));
		setEditedContentText("");
		setMultiEditContentTexts([]);
		setEditedPreviewInput(null);
		setContentEditError(null);
		setPreviewEditError(null);
	}, [request.input]);
	// inputKey は request.input の内容ハッシュ。参照ではなく内容で再取得を判定するため、
	// request.input そのものではなく inputKey を依存にする（チラつき防止）。
	// biome-ignore lint/correctness/useExhaustiveDependencies: request.input は inputKey 経由で内容監視している
	useEffect(() => {
		let canceled = false;
		setPresentation(emptyPermissionPresentation());
		setPresentationReady(false);
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
				setPresentationReady(true);
			})
			.catch(() => {
				if (canceled) return;
				setPresentation(emptyPermissionPresentation());
				setEditedContentText("");
				setMultiEditContentTexts([]);
				setPresentationReady(true);
			});
		return () => {
			canceled = true;
		};
	}, [inputKey, request.tool_name]);
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
	// presentation は Rust から非同期取得するため、確定するまでは kind 依存の
	// 分岐 UI を描画しない。確定前に汎用 UI を出すと、解決後に別 UI へ差し替わって
	// チラつく（特に仮想化リストの再マウント時）。
	if (!presentationReady) {
		return (
			<PermissionShell data-testid="permission-loading">
				<div className="px-2 py-1">
					<div className="h-4 w-2/3 animate-pulse rounded bg-muted-foreground/10" />
				</div>
			</PermissionShell>
		);
	}
	if (status !== "pending") {
		const isAllowed = status === "allowed";
		let label: string;
		if (presentation.kind === "exit_plan") {
			label = isAllowed ? "Plan approved" : "Plan denied";
		} else {
			const toolLabel =
				request.title || request.display_name || request.tool_name;
			label = `${toolLabel} — ${status}`;
		}

		const hasDetail = presentation.hasResolvedDetail;
		const answerSummary = resolvedAnswers
			? Object.values(resolvedAnswers).join(", ")
			: "";
		const firstQuestion =
			presentation.kind === "ask_user_question"
				? (presentation.questions[0]?.question ??
					request.description ??
					"Question")
				: "";

		if (presentation.kind === "ask_user_question") {
			return (
				<div data-testid="permission-resolved">
					{/* Agent の発言: 質問＋選択肢ウィジェット（原状） */}
					<PermissionShell>
						<div className="flex min-w-0 items-start gap-2 px-2 py-1 text-muted-foreground">
							<PermissionKindIcon kind={presentation.kind} />
							<div className="min-w-0 flex-1">
								<InlineMarkdown className="text-foreground">
									{firstQuestion}
								</InlineMarkdown>
								{hasDetail && (
									<button
										type="button"
										className="mt-1 inline-flex items-center gap-1 text-muted-foreground hover:text-foreground"
										onClick={() => setIsExpanded(!isExpanded)}
									>
										<ChevronRight
											className={cn(
												"size-3 shrink-0 transition-transform",
												isExpanded && "rotate-90",
											)}
										/>
										Choices
									</button>
								)}
								{isExpanded && (
									<ResolvedDetail
										request={request}
										presentation={presentation}
										resolvedAnswers={resolvedAnswers}
									/>
								)}
							</div>
							{isAllowed ? (
								<Check className="size-3.5 shrink-0" />
							) : (
								<X className="size-3.5 shrink-0" />
							)}
						</div>
					</PermissionShell>
					{/* ユーザーの発言: 選択した内容を分離表示 */}
					{answerSummary && (
						<div className="flex justify-end px-4 py-1">
							<div className="max-w-[min(82%,48rem)]">
								<UserMessage content={answerSummary} copyLabel="Copy answer" />
							</div>
						</div>
					)}
				</div>
			);
		}

		return (
			<PermissionShell data-testid="permission-resolved">
				<button
					type="button"
					className="flex w-full min-w-0 items-center gap-2 px-2 py-1 text-left text-muted-foreground hover:text-foreground"
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
					<PermissionKindIcon kind={presentation.kind} />
					<span className="min-w-0 flex-1 truncate">{label}</span>
					{isAllowed ? (
						<Check className="size-3.5 shrink-0" />
					) : (
						<X className="size-3.5 shrink-0" />
					)}
				</button>
				{isExpanded && (
					<ResolvedDetail
						request={request}
						presentation={presentation}
						resolvedAnswers={resolvedAnswers}
					/>
				)}
			</PermissionShell>
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
			<PermissionShell>
				<div className="mb-2 flex items-center gap-2 px-2 text-muted-foreground">
					<PermissionKindIcon kind={presentation.kind} />
					<span>Question</span>
				</div>
				{questions.map((q, qIndex) => {
					const questionId = `${questionIdBase}-q-${qIndex}`;
					return (
						<div key={q.question} className="mb-2 px-2">
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
												className={cn(
													"flex cursor-pointer items-start gap-2.5 rounded px-2 py-1.5 hover:bg-foreground/5",
													isChecked && "bg-foreground/5",
												)}
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
									{q.options.map((opt) => {
										const isSelected = answers[q.question] === opt.label;
										return (
											// biome-ignore lint/a11y/noLabelWithoutControl: Radix RadioGroupItem renders an internal button element
											<label
												key={opt.label}
												className={cn(
													"flex cursor-pointer items-start gap-2.5 rounded px-2 py-1.5 hover:bg-foreground/5",
													isSelected && "bg-foreground/5",
												)}
											>
												<RadioGroupItem value={opt.label} className="mt-0.5" />
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
									{/* biome-ignore lint/a11y/noLabelWithoutControl: Radix RadioGroupItem renders an internal button element */}
									<label
										className={cn(
											"flex cursor-pointer items-start gap-2.5 rounded px-2 py-1.5 hover:bg-foreground/5",
											answers[q.question] === OTHER_LABEL && "bg-foreground/5",
										)}
									>
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
					className="mt-1 ml-2"
				>
					Submit
				</Button>
			</PermissionShell>
		);
	}

	if (presentation.kind === "exit_plan") {
		const { plan, allowedPrompts } = presentation;

		return (
			<PermissionShell>
				<div className="mb-2 flex items-center gap-2 px-2 text-sm font-medium">
					<PermissionKindIcon kind={presentation.kind} />
					<span>Plan Review</span>
				</div>
				<div className="mb-2 space-y-2 px-2">
					<PlanContent plan={plan} allowedPrompts={allowedPrompts} />
				</div>
				<AllowDenyButtons
					requestId={request.request_id}
					onAllow={onAllow}
					onDeny={onDeny}
				/>
			</PermissionShell>
		);
	}

	const toolLabel = request.title || request.display_name || request.tool_name;

	return (
		<PermissionShell>
			<div className="mb-1 flex min-w-0 items-center gap-2 px-2 text-sm font-medium">
				<PermissionKindIcon kind={presentation.kind} />
				<span className="min-w-0 truncate">
					Permission required: {toolLabel}
				</span>
			</div>
			{request.description && (
				<p className="mb-2 px-2 text-xs text-muted-foreground">
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
								onOpenDiffFile={onOpenDiffFile}
							/>
							{previewEditError && (
								<p className="mt-1 px-2 text-xs text-destructive">
									Edited preview unavailable: {previewEditError}
								</p>
							)}
						</div>
					)}
					<pre
						data-testid="permission-input"
						className="mx-2 mb-2 max-h-32 overflow-y-auto whitespace-pre-wrap break-all rounded bg-muted/40 p-2 text-xs"
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
												className="rounded border border-border/60 p-2"
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
		</PermissionShell>
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
		<div className="flex gap-2 px-2">
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
