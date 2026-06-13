import { invoke } from "@tauri-apps/api/core";
import {
	ArrowUp,
	ChevronDown,
	ChevronUp,
	ExternalLink,
	FileInput,
	Maximize2,
	Mic,
	Search,
	Square,
	X,
} from "lucide-react";
import {
	useCallback,
	useEffect,
	useImperativeHandle,
	useMemo,
	useRef,
	useState,
} from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import type {
	AgentSkill,
	BackendInfo,
	ImageAttachment,
	MentionReference,
	ModelInfo,
	PermissionMode,
	SlashCommand,
} from "@/types/session";
import { BackendSelector } from "./BackendSelector";
import { MentionPopup } from "./MentionPopup";
import { ModelSelector } from "./ModelSelector";
import { ModeSelector } from "./ModeSelector";
import {
	findMentionTrigger,
	findSkillTrigger,
	handlePopupKeyDown,
} from "./popupInputUtils";
import { SkillPopup } from "./SkillPopup";
import { SlashCommandPopup } from "./SlashCommandPopup";

function mentionTokenText(filePath: string): string {
	if (!/[\s"]/.test(filePath)) return `@${filePath}`;
	return `@"${filePath.replace(/["\\]/g, "\\$&")}"`;
}

interface AttachedImage {
	id: string;
	attachment: ImageAttachment;
	previewUrl: string;
}

interface PastedTextBlock {
	id: number;
	placeholder: string;
	content: string;
}

interface PromptEditorDraftInfo {
	id: string;
	filePath: string;
}

interface ShellCompletionResult {
	completed?: string | null;
}

interface AgentDictationPresentation {
	enabled: boolean;
	label: string;
	title: string;
	status?: string | null;
}

interface AgentDictationDraft {
	value: string;
	caret: number;
}

interface SpeechRecognitionAlternativeLike {
	transcript: string;
}

interface SpeechRecognitionResultLike {
	isFinal: boolean;
	readonly length: number;
	[index: number]: SpeechRecognitionAlternativeLike | undefined;
}

interface SpeechRecognitionResultListLike {
	readonly length: number;
	[index: number]: SpeechRecognitionResultLike | undefined;
}

interface SpeechRecognitionEventLike {
	results: SpeechRecognitionResultListLike;
}

interface SpeechRecognitionErrorEventLike {
	error?: string;
	message?: string;
}

interface SpeechRecognitionLike {
	continuous: boolean;
	interimResults: boolean;
	lang: string;
	onresult: ((event: SpeechRecognitionEventLike) => void) | null;
	onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
	onend: (() => void) | null;
	start: () => void;
	stop: () => void;
	abort: () => void;
}

type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

function getSpeechRecognitionConstructor(): SpeechRecognitionConstructor | null {
	const speechWindow = window as Window &
		typeof globalThis & {
			SpeechRecognition?: SpeechRecognitionConstructor;
			webkitSpeechRecognition?: SpeechRecognitionConstructor;
		};
	return (
		speechWindow.SpeechRecognition ??
		speechWindow.webkitSpeechRecognition ??
		null
	);
}

function isAgentDictationPresentation(
	value: unknown,
): value is AgentDictationPresentation {
	return (
		typeof value === "object" &&
		value !== null &&
		typeof (value as AgentDictationPresentation).enabled === "boolean" &&
		typeof (value as AgentDictationPresentation).label === "string" &&
		typeof (value as AgentDictationPresentation).title === "string"
	);
}

type PromptHistoryScope = "session" | "project" | "all";

interface PromptHistoryMatch {
	text: string;
	scope: PromptHistoryScope;
	index?: number;
}

interface AgentPromptHistoryEntry {
	text: string;
	scope: PromptHistoryScope;
	sessionId?: string | null;
	worktreePath?: string | null;
	timestamp: number;
}

export interface MessageInputHandle {
	addImageAttachments: (attachments: ImageAttachment[]) => void;
	setDraft: (content: string) => void;
	getDraft: () => string;
	clearDraft: () => void;
}

interface MessageInputProps {
	onSend: (
		content: string,
		images?: ImageAttachment[],
		mentions?: MentionReference[],
	) => void | Promise<void>;
	onInterrupt: () => void;
	isStreaming: boolean;
	onCycleMode?: () => void;
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	models: ModelInfo[];
	currentModelId: string;
	onModelChange: (modelId: string) => void;
	backends: BackendInfo[];
	currentBackendId: string | null;
	onBackendChange: (backendId: string | null) => void;
	backendDisabled: boolean;
	ref?: React.Ref<MessageInputHandle>;
	worktreePath?: string;
	chatSessionId?: string;
	promptSuggestion?: string | null;
	runtimeSlashCommands?: SlashCommand[];
}

export function MessageInput({
	onSend,
	onInterrupt,
	isStreaming,
	onCycleMode,
	mode,
	onModeChange,
	models,
	currentModelId,
	onModelChange,
	backends,
	currentBackendId,
	onBackendChange,
	backendDisabled,
	ref,
	worktreePath,
	chatSessionId,
	promptSuggestion,
	runtimeSlashCommands = [],
}: MessageInputProps) {
	const [value, setValue] = useState("");
	const textareaRef = useRef<HTMLTextAreaElement>(null);
	const historySearchInputRef = useRef<HTMLInputElement>(null);
	const [slashPopupDismissed, setSlashPopupDismissed] = useState(false);
	const [selectedIndex, setSelectedIndex] = useState(0);
	const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
	const imageIdCounterRef = useRef(0);
	const pastedTextIdCounterRef = useRef(0);
	const [pastedTextBlocks, setPastedTextBlocks] = useState<PastedTextBlock[]>(
		[],
	);
	const [promptHistory, setPromptHistory] = useState<string[]>([]);
	const [historyIndex, setHistoryIndex] = useState<number | null>(null);
	const [historySearchOpen, setHistorySearchOpen] = useState(false);
	const [historySearchQuery, setHistorySearchQuery] = useState("");
	const [historySearchScope, setHistorySearchScope] =
		useState<PromptHistoryScope>("session");
	const [rustHistorySearchMatches, setRustHistorySearchMatches] = useState<
		PromptHistoryMatch[]
	>([]);
	const [historySearchSelectedIndex, setHistorySearchSelectedIndex] =
		useState(0);
	const [promptEditorOpen, setPromptEditorOpen] = useState(false);
	const [promptEditorValue, setPromptEditorValue] = useState("");
	const promptEditorRef = useRef<HTMLTextAreaElement>(null);
	const [externalPromptDraftId, setExternalPromptDraftId] = useState<
		string | null
	>(null);
	const [promptEditorError, setPromptEditorError] = useState<string | null>(
		null,
	);
	const [isOpeningExternalEditor, setIsOpeningExternalEditor] = useState(false);
	const [isImportingExternalEditor, setIsImportingExternalEditor] =
		useState(false);
	const dictationRecognitionRef = useRef<SpeechRecognitionLike | null>(null);
	const dictationBaseValueRef = useRef("");
	const [dictationSupported, setDictationSupported] = useState(false);
	const [dictationListening, setDictationListening] = useState(false);
	const [dictationError, setDictationError] = useState<string | null>(null);
	const [dictationPresentation, setDictationPresentation] =
		useState<AgentDictationPresentation>({
			enabled: false,
			label: "Dictation unavailable",
			title: "Voice dictation is unavailable",
			status: null,
		});

	// Mention state
	const [mentionDismissed, setMentionDismissed] = useState(false);
	const [mentionSelectedIndex, setMentionSelectedIndex] = useState(0);
	const [mentionFiles, setMentionFiles] = useState<string[]>([]);
	const [mentionTrigger, setMentionTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);
	const [mentionRefs, setMentionRefs] = useState<MentionReference[]>([]);
	const [skillDismissed, setSkillDismissed] = useState(false);
	const [skillSelectedIndex, setSkillSelectedIndex] = useState(0);
	const [skillCandidates, setSkillCandidates] = useState<AgentSkill[]>([]);
	const [skillTrigger, setSkillTrigger] = useState<{
		start: number;
		query: string;
	} | null>(null);

	const allCommands = useMemo(() => {
		const seen = new Set<string>();
		const merged: SlashCommand[] = [];
		for (const command of runtimeSlashCommands) {
			if (seen.has(command.name)) continue;
			seen.add(command.name);
			merged.push(command);
		}
		return merged;
	}, [runtimeSlashCommands]);
	const setComposerValue = useCallback((nextValue: string) => {
		setValue(nextValue);
		requestAnimationFrame(() => {
			const el = textareaRef.current;
			if (!el) return;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
			el.setSelectionRange(nextValue.length, nextValue.length);
		});
	}, []);
	const setComposerValueWithCaret = useCallback(
		(nextValue: string, caret: number) => {
			setValue(nextValue);
			requestAnimationFrame(() => {
				const el = textareaRef.current;
				if (!el) return;
				el.style.height = "auto";
				el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
				el.setSelectionRange(caret, caret);
			});
		},
		[],
	);

	useEffect(() => {
		setDictationSupported(Boolean(getSpeechRecognitionConstructor()));
	}, []);

	useEffect(() => {
		let cancelled = false;
		void Promise.resolve(
			invoke<AgentDictationPresentation>("present_agent_dictation", {
				request: {
					supported: dictationSupported,
					listening: dictationListening,
					error: dictationError,
				},
			}),
		)
			.then((presentation) => {
				if (!cancelled) {
					if (isAgentDictationPresentation(presentation)) {
						setDictationPresentation(presentation);
					}
				}
			})
			.catch(() => {
				if (!cancelled) {
					setDictationPresentation({
						enabled: dictationSupported,
						label: dictationListening ? "Stop dictation" : "Start dictation",
						title: dictationListening
							? "Stop voice dictation"
							: "Start voice dictation",
						status: dictationError,
					});
				}
			});
		return () => {
			cancelled = true;
		};
	}, [dictationError, dictationListening, dictationSupported]);

	const applyDictationTranscript = useCallback(
		async (transcript: string) => {
			const result = await invoke<AgentDictationDraft>(
				"compose_agent_dictation_draft",
				{
					request: {
						baseValue: dictationBaseValueRef.current,
						transcript,
					},
				},
			);
			setComposerValueWithCaret(result.value, result.caret);
			setHistoryIndex(null);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);
		},
		[setComposerValueWithCaret],
	);

	const createAttachedImage = useCallback(
		(attachment: ImageAttachment): AttachedImage => {
			return {
				id: `img-${++imageIdCounterRef.current}`,
				attachment,
				previewUrl: `data:${attachment.mediaType};base64,${attachment.data}`,
			};
		},
		[],
	);

	const processImageFile = useCallback(
		async (file: File): Promise<AttachedImage | null> => {
			const bytes = new Uint8Array(await file.arrayBuffer());
			try {
				const attachment = await invoke<ImageAttachment>(
					"prepare_image_attachment",
					{ data: Array.from(bytes) },
				);
				return createAttachedImage(attachment);
			} catch {
				return null;
			}
		},
		[createAttachedImage],
	);

	useImperativeHandle(
		ref,
		() => ({
			addImageAttachments: (attachments: ImageAttachment[]) => {
				const newImages = attachments.map(createAttachedImage);
				setAttachedImages((prev) => [...prev, ...newImages]);
			},
			setDraft: (content: string) => {
				setComposerValue(content);
				setHistoryIndex(null);
				setSlashPopupDismissed(false);
				requestAnimationFrame(() => textareaRef.current?.focus());
			},
			getDraft: () => value,
			clearDraft: () => {
				setValue("");
				setAttachedImages([]);
				setPastedTextBlocks([]);
				setMentionRefs([]);
				setSlashPopupDismissed(false);
				setSelectedIndex(0);
				setMentionTrigger(null);
				setMentionDismissed(false);
				setMentionSelectedIndex(0);
				setSkillTrigger(null);
				setSkillDismissed(false);
				setSkillSelectedIndex(0);
				requestAnimationFrame(() => textareaRef.current?.focus());
			},
		}),
		[createAttachedImage, setComposerValue, value],
	);

	const showSlashPopup =
		value.startsWith("/") && !value.includes(" ") && !slashPopupDismissed;
	const slashQuery = value.slice(1).toLowerCase();

	const filteredCommands = useMemo(() => {
		if (!showSlashPopup) return [];
		if (slashQuery === "") return allCommands;
		return allCommands.filter((cmd) =>
			cmd.name.toLowerCase().startsWith(slashQuery),
		);
	}, [showSlashPopup, slashQuery, allCommands]);
	const localHistorySearchMatches = useMemo<PromptHistoryMatch[]>(() => {
		const query = historySearchQuery.trim().toLowerCase();
		return promptHistory
			.map((text, index) => ({ text, index }))
			.filter(({ text }) => !query || text.toLowerCase().includes(query))
			.reverse()
			.map((match) => ({ ...match, scope: "session" as const }));
	}, [promptHistory, historySearchQuery]);
	const usesRustHistorySearch = Boolean(chatSessionId && worktreePath);
	const historySearchMatches = usesRustHistorySearch
		? rustHistorySearchMatches
		: localHistorySearchMatches;

	const popupOpen = showSlashPopup && filteredCommands.length > 0;
	const mentionPopupOpen =
		!mentionDismissed && mentionTrigger !== null && mentionFiles.length > 0;
	const mentionQuery = mentionTrigger?.query ?? null;
	const skillPopupOpen =
		!skillDismissed && skillTrigger !== null && skillCandidates.length > 0;
	const skillQuery = skillTrigger?.query ?? null;
	const activePromptSuggestion =
		value.trim().length === 0 && attachedImages.length === 0
			? promptSuggestion?.trim() || null
			: null;
	const activeSlashArgumentHelp = useMemo(() => {
		const match = value.match(/^\/([^\s]+)\s*$/);
		if (!match || !value.endsWith(" ")) return null;
		const command = allCommands.find((cmd) => cmd.name === match[1]);
		if (!command?.argumentHint) return null;
		return command;
	}, [allCommands, value]);

	// Fetch mention candidates when trigger changes (debounced)
	useEffect(() => {
		if (mentionQuery === null || mentionDismissed || !worktreePath) {
			setMentionFiles([]);
			return;
		}

		setMentionFiles([]);
		setMentionSelectedIndex(0);

		let cancelled = false;
		const timer = setTimeout(() => {
			const mentionRequest =
				currentBackendId === "codex"
					? invoke<string[]>("read_codex_mentionable_files", {
							worktreePath,
							query: mentionQuery,
						}).catch(() =>
							invoke<string[]>("list_mentionable_files", {
								worktreePath,
								query: mentionQuery,
							}),
						)
					: invoke<string[]>("list_mentionable_files", {
							worktreePath,
							query: mentionQuery,
						});
			mentionRequest
				.then((files) => {
					if (!cancelled) {
						setMentionFiles(files);
						setMentionSelectedIndex(0);
					}
				})
				.catch(() => {
					if (!cancelled) {
						setMentionFiles([]);
					}
				});
		}, 150);

		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [mentionQuery, mentionDismissed, worktreePath, currentBackendId]);

	useEffect(() => {
		if (skillQuery === null || skillDismissed || !worktreePath) {
			setSkillCandidates([]);
			return;
		}

		setSkillCandidates([]);
		setSkillSelectedIndex(0);

		let cancelled = false;
		const normalizedQuery = skillQuery.trim().toLowerCase();
		const timer = setTimeout(() => {
			const command =
				currentBackendId === "codex"
					? "read_codex_skill_catalog"
					: "scan_agent_skills";
			invoke<AgentSkill[]>(command, {
				cwd: worktreePath,
				query: normalizedQuery,
				limit: 20,
				...(command === "scan_agent_skills"
					? { backendId: currentBackendId ?? undefined }
					: {}),
			})
				.then((skills) => {
					if (cancelled) return;
					setSkillCandidates(skills);
					setSkillSelectedIndex(0);
				})
				.catch(() => {
					if (!cancelled) {
						setSkillCandidates([]);
					}
				});
		}, 150);

		return () => {
			cancelled = true;
			clearTimeout(timer);
		};
	}, [skillQuery, skillDismissed, worktreePath, currentBackendId]);

	useEffect(() => {
		if (!historySearchOpen) return;
		setHistorySearchSelectedIndex((current) =>
			Math.min(current, Math.max(historySearchMatches.length - 1, 0)),
		);
	}, [historySearchMatches.length, historySearchOpen]);

	useEffect(() => {
		if (!historySearchOpen || !chatSessionId || !worktreePath) {
			setRustHistorySearchMatches([]);
			return;
		}
		let cancelled = false;
		const timer = window.setTimeout(() => {
			invoke<AgentPromptHistoryEntry[]>("search_agent_prompt_history", {
				request: {
					chatSessionId,
					worktreePath,
					query: historySearchQuery,
					scope: historySearchScope,
					localHistory: promptHistory,
					limit: 30,
				},
			})
				.then((entries) => {
					if (cancelled) return;
					setRustHistorySearchMatches(
						entries.map((entry) => ({
							text: entry.text,
							scope: entry.scope,
						})),
					);
					setHistorySearchSelectedIndex(0);
				})
				.catch(() => {
					if (cancelled) return;
					setRustHistorySearchMatches(localHistorySearchMatches);
				});
		}, 120);
		return () => {
			cancelled = true;
			window.clearTimeout(timer);
		};
	}, [
		chatSessionId,
		historySearchOpen,
		historySearchQuery,
		historySearchScope,
		localHistorySearchMatches,
		promptHistory,
		worktreePath,
	]);

	const handleSelectCommand = useCallback((cmd: SlashCommand) => {
		setValue(`/${cmd.name} `);
		setSlashPopupDismissed(true);
		setSelectedIndex(0);
		if (textareaRef.current) {
			textareaRef.current.focus();
		}
	}, []);

	const openHistorySearch = useCallback(() => {
		if (!usesRustHistorySearch && promptHistory.length === 0) return;
		setHistorySearchQuery(value);
		setHistorySearchSelectedIndex(0);
		setHistorySearchOpen(true);
		requestAnimationFrame(() => {
			historySearchInputRef.current?.focus();
			historySearchInputRef.current?.select();
		});
	}, [promptHistory.length, usesRustHistorySearch, value]);

	const closeHistorySearch = useCallback(() => {
		setHistorySearchOpen(false);
		requestAnimationFrame(() => {
			textareaRef.current?.focus();
		});
	}, []);

	const acceptHistorySearchMatch = useCallback(() => {
		const match = historySearchMatches[historySearchSelectedIndex];
		if (!match) return;
		setHistoryIndex(match.index ?? null);
		setComposerValue(match.text);
		closeHistorySearch();
	}, [
		closeHistorySearch,
		historySearchMatches,
		historySearchSelectedIndex,
		setComposerValue,
	]);

	const openPromptEditor = useCallback(() => {
		setPromptEditorValue(value);
		setPromptEditorError(null);
		setPromptEditorOpen(true);
		requestAnimationFrame(() => {
			const editor = promptEditorRef.current;
			if (!editor) return;
			editor.focus();
			editor.setSelectionRange(editor.value.length, editor.value.length);
		});
	}, [value]);

	const discardExternalPromptDraft = useCallback((draftId: string | null) => {
		if (!draftId) return;
		invoke("discard_agent_prompt_external_editor_draft", {
			draftId,
		}).catch((err) => {
			console.error("Failed to discard external prompt draft:", err);
		});
	}, []);

	const handlePromptEditorOpenChange = useCallback(
		(open: boolean) => {
			setPromptEditorOpen(open);
			if (open) return;
			discardExternalPromptDraft(externalPromptDraftId);
			setExternalPromptDraftId(null);
			setPromptEditorError(null);
		},
		[discardExternalPromptDraft, externalPromptDraftId],
	);

	const applyPromptEditorValue = useCallback(() => {
		setComposerValue(promptEditorValue);
		setHistoryIndex(null);
		setSlashPopupDismissed(false);
		setSelectedIndex(0);
		setPromptEditorOpen(false);
		discardExternalPromptDraft(externalPromptDraftId);
		setExternalPromptDraftId(null);
	}, [
		discardExternalPromptDraft,
		externalPromptDraftId,
		promptEditorValue,
		setComposerValue,
	]);

	const completeShellDraft = useCallback(async () => {
		const result = await invoke<ShellCompletionResult>(
			"complete_agent_shell_command",
			{
				history: promptHistory,
				draft: value,
			},
		);
		if (!result.completed) return false;
		setComposerValue(result.completed);
		setHistoryIndex(null);
		return true;
	}, [promptHistory, setComposerValue, value]);

	const openExternalPromptEditor = useCallback(async () => {
		setPromptEditorError(null);
		setIsOpeningExternalEditor(true);
		try {
			const previousDraftId = externalPromptDraftId;
			const draft = await invoke<PromptEditorDraftInfo>(
				"open_agent_prompt_in_external_editor",
				{ content: promptEditorValue },
			);
			discardExternalPromptDraft(previousDraftId);
			setExternalPromptDraftId(draft.id);
		} catch (err) {
			setPromptEditorError(String(err));
		} finally {
			setIsOpeningExternalEditor(false);
		}
	}, [discardExternalPromptDraft, externalPromptDraftId, promptEditorValue]);

	const importExternalPromptEdits = useCallback(async () => {
		if (!externalPromptDraftId) return;
		setPromptEditorError(null);
		setIsImportingExternalEditor(true);
		try {
			const content = await invoke<string>(
				"import_agent_prompt_external_editor_draft",
				{ draftId: externalPromptDraftId },
			);
			setPromptEditorValue(content);
			setExternalPromptDraftId(null);
			requestAnimationFrame(() => {
				const editor = promptEditorRef.current;
				if (!editor) return;
				editor.focus();
				editor.setSelectionRange(editor.value.length, editor.value.length);
			});
		} catch (err) {
			setPromptEditorError(String(err));
		} finally {
			setIsImportingExternalEditor(false);
		}
	}, [externalPromptDraftId]);

	const stopDictation = useCallback(() => {
		const recognition = dictationRecognitionRef.current;
		if (!recognition) return;
		recognition.stop();
	}, []);

	const startDictation = useCallback(() => {
		if (dictationRecognitionRef.current) return;
		const Recognition = getSpeechRecognitionConstructor();
		if (!Recognition) {
			setDictationError("Voice dictation is unavailable in this WebView");
			return;
		}

		const recognition = new Recognition();
		recognition.continuous = true;
		recognition.interimResults = true;
		recognition.lang = navigator.language || "en-US";
		dictationBaseValueRef.current = value;
		setDictationError(null);

		recognition.onresult = (event) => {
			const parts: string[] = [];
			for (let index = 0; index < event.results.length; index++) {
				const result = event.results[index];
				const transcript = result?.[0]?.transcript;
				if (transcript) {
					parts.push(transcript);
				}
			}
			const transcript = parts.join(" ").replace(/\s+/g, " ").trim();
			void applyDictationTranscript(transcript);
		};
		recognition.onerror = (event) => {
			setDictationError(event.message || event.error || "Dictation failed");
			setDictationListening(false);
		};
		recognition.onend = () => {
			dictationRecognitionRef.current = null;
			setDictationListening(false);
			requestAnimationFrame(() => textareaRef.current?.focus());
		};

		try {
			dictationRecognitionRef.current = recognition;
			setDictationListening(true);
			recognition.start();
		} catch (err) {
			dictationRecognitionRef.current = null;
			setDictationListening(false);
			setDictationError(String(err));
		}
	}, [applyDictationTranscript, value]);

	const toggleDictation = useCallback(() => {
		if (dictationListening) {
			stopDictation();
		} else {
			startDictation();
		}
	}, [dictationListening, startDictation, stopDictation]);

	useEffect(() => {
		if (!dictationListening) return;
		const handleKeyUp = (event: KeyboardEvent) => {
			if (event.key.toLowerCase() === "m") {
				stopDictation();
			}
		};
		window.addEventListener("keyup", handleKeyUp);
		return () => window.removeEventListener("keyup", handleKeyUp);
	}, [dictationListening, stopDictation]);

	useEffect(() => {
		return () => {
			dictationRecognitionRef.current?.abort();
		};
	}, []);

	const cycleHistorySearchScope = useCallback(() => {
		setHistorySearchScope((current) => {
			switch (current) {
				case "session":
					return "project";
				case "project":
					return "all";
				case "all":
					return "session";
			}
		});
		setHistorySearchSelectedIndex(0);
	}, []);

	const addImages = useCallback(
		async (files: File[]) => {
			const results = await Promise.all(files.map(processImageFile));
			const valid = results.filter((r): r is AttachedImage => r !== null);
			if (valid.length > 0) {
				setAttachedImages((prev) => [...prev, ...valid]);
			}
		},
		[processImageFile],
	);

	const removeImage = useCallback((id: string) => {
		setAttachedImages((prev) => prev.filter((img) => img.id !== id));
	}, []);

	const expandSubmittedContent = useCallback(
		async (content: string): Promise<string> => {
			if (pastedTextBlocks.length === 0) return content;
			return invoke<string>("expand_pasted_text_blocks", {
				content,
				blocks: pastedTextBlocks,
			});
		},
		[pastedTextBlocks],
	);
	const syncMentionsForSubmit = useCallback(
		async (content: string): Promise<MentionReference[] | undefined> => {
			if (mentionRefs.length === 0) return undefined;
			return invoke<MentionReference[] | null>("sync_mentions_with_text", {
				text: content,
				refs: mentionRefs,
			}).then((mentions) => mentions ?? undefined);
		},
		[mentionRefs],
	);

	const handleSelectMention = useCallback(
		(filePath: string) => {
			if (!mentionTrigger) return;
			const token = mentionTokenText(filePath);
			const before = value.slice(0, mentionTrigger.start);
			const replacementEnd =
				textareaRef.current?.selectionStart ??
				mentionTrigger.start + 1 + mentionTrigger.query.length;
			const after = value.slice(replacementEnd).replace(/^\s/, "");
			const newValue = `${before}${token} ${after}`;
			setValue(newValue);
			setMentionRefs((prev) => [
				...prev,
				{ filePath, startLine: undefined, endLine: undefined },
			]);
			setMentionTrigger(null);
			setMentionDismissed(true);
			setMentionSelectedIndex(0);
			const caret = before.length + token.length + 1;
			const el = textareaRef.current;
			if (el) {
				el.focus();
				requestAnimationFrame(() => {
					el.setSelectionRange(caret, caret);
				});
			}
		},
		[mentionTrigger, value],
	);

	const handleSelectSkill = useCallback(
		(skill: AgentSkill) => {
			if (!skillTrigger) return;
			const before = value.slice(0, skillTrigger.start);
			const after = value
				.slice(skillTrigger.start + 1 + skillTrigger.query.length)
				.replace(/^\s/, "");
			const token = `$${skill.name}`;
			const newValue = `${before}${token} ${after}`;
			setValue(newValue);
			setSkillTrigger(null);
			setSkillDismissed(true);
			setSkillSelectedIndex(0);
			const caret = before.length + token.length + 1;
			const el = textareaRef.current;
			if (el) {
				el.focus();
				requestAnimationFrame(() => {
					el.setSelectionRange(caret, caret);
				});
			}
		},
		[skillTrigger, value],
	);

	const handleSubmit = useCallback(() => {
		const trimmed = value.trim();
		const hasImages = attachedImages.length > 0;
		if (!trimmed && !hasImages) return;

		const submitContent = async (submittedContent: string) => {
			const currentMentions =
				mentionRefs.length === 0
					? undefined
					: await syncMentionsForSubmit(submittedContent);
			if (hasImages) {
				onSend(
					submittedContent,
					attachedImages.map((img) => img.attachment),
					currentMentions,
				);
			} else {
				onSend(submittedContent, undefined, currentMentions);
			}
			if (trimmed) {
				setPromptHistory((current) =>
					current[current.length - 1] === trimmed
						? current
						: [...current, trimmed],
				);
			}
			setHistoryIndex(null);
			setValue("");
			setAttachedImages([]);
			setPastedTextBlocks([]);
			setMentionRefs([]);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);
			setMentionTrigger(null);
			setMentionDismissed(false);
			setMentionSelectedIndex(0);
			setSkillTrigger(null);
			setSkillDismissed(false);
			setSkillSelectedIndex(0);
			if (textareaRef.current) {
				textareaRef.current.style.height = "auto";
			}
		};

		if (pastedTextBlocks.length > 0) {
			void expandSubmittedContent(trimmed)
				.then(submitContent)
				.catch((e) => {
					console.error("Failed to expand pasted text blocks:", e);
				});
			return;
		}
		void submitContent(trimmed);
	}, [
		value,
		onSend,
		attachedImages,
		expandSubmittedContent,
		mentionRefs.length,
		syncMentionsForSubmit,
		pastedTextBlocks.length,
	]);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLTextAreaElement>) => {
			// Mention popup keyboard handling (takes priority when open)
			if (mentionPopupOpen) {
				if (
					handlePopupKeyDown(
						e,
						mentionFiles.length,
						setMentionSelectedIndex,
						() => {
							if (mentionFiles[mentionSelectedIndex]) {
								handleSelectMention(mentionFiles[mentionSelectedIndex]);
							}
						},
						() => setMentionDismissed(true),
					)
				)
					return;
			}

			if (skillPopupOpen) {
				if (
					handlePopupKeyDown(
						e,
						skillCandidates.length,
						setSkillSelectedIndex,
						() => {
							if (skillCandidates[skillSelectedIndex]) {
								handleSelectSkill(skillCandidates[skillSelectedIndex]);
							}
						},
						() => setSkillDismissed(true),
					)
				)
					return;
			}

			// Slash command popup keyboard handling
			if (popupOpen) {
				if (
					handlePopupKeyDown(
						e,
						filteredCommands.length,
						setSelectedIndex,
						() => {
							if (filteredCommands[selectedIndex]) {
								handleSelectCommand(filteredCommands[selectedIndex]);
							}
						},
						() => setSlashPopupDismissed(true),
					)
				)
					return;
			}

			if (e.key.toLowerCase() === "m" && e.ctrlKey && !e.metaKey && !e.altKey) {
				e.preventDefault();
				startDictation();
				return;
			}
			if (e.key === "Tab" && e.shiftKey) {
				e.preventDefault();
				onCycleMode?.();
				return;
			}
			if (
				e.key === "Tab" &&
				value.trimStart().startsWith("!") &&
				!e.shiftKey &&
				!e.metaKey &&
				!e.ctrlKey &&
				!e.altKey
			) {
				e.preventDefault();
				void completeShellDraft();
				return;
			}
			if (
				activePromptSuggestion &&
				(e.key === "Tab" || e.key === "ArrowRight") &&
				!e.shiftKey &&
				!e.metaKey &&
				!e.ctrlKey &&
				!e.altKey
			) {
				e.preventDefault();
				setComposerValue(activePromptSuggestion);
				return;
			}
			if (e.key.toLowerCase() === "r" && e.ctrlKey && !e.metaKey && !e.altKey) {
				e.preventDefault();
				openHistorySearch();
				return;
			}
			if (
				isStreaming &&
				value.length === 0 &&
				e.key.toLowerCase() === "c" &&
				e.ctrlKey &&
				!e.metaKey &&
				!e.altKey
			) {
				e.preventDefault();
				onInterrupt();
				return;
			}
			if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
				e.preventDefault();
				handleSubmit();
				return;
			}
			if (
				e.key === "ArrowUp" &&
				!e.metaKey &&
				!e.ctrlKey &&
				!e.altKey &&
				promptHistory.length > 0 &&
				(value.length === 0 || historyIndex !== null)
			) {
				e.preventDefault();
				const nextIndex =
					historyIndex === null
						? promptHistory.length - 1
						: Math.max(0, historyIndex - 1);
				setHistoryIndex(nextIndex);
				setComposerValue(promptHistory[nextIndex]);
				return;
			}
			if (
				e.key === "ArrowDown" &&
				!e.metaKey &&
				!e.ctrlKey &&
				!e.altKey &&
				historyIndex !== null
			) {
				e.preventDefault();
				const nextIndex = historyIndex + 1;
				if (nextIndex >= promptHistory.length) {
					setHistoryIndex(null);
					setComposerValue("");
				} else {
					setHistoryIndex(nextIndex);
					setComposerValue(promptHistory[nextIndex]);
				}
			}
		},
		[
			handleSubmit,
			onCycleMode,
			popupOpen,
			filteredCommands,
			selectedIndex,
			handleSelectCommand,
			openHistorySearch,
			mentionPopupOpen,
			mentionFiles,
			mentionSelectedIndex,
			handleSelectMention,
			skillPopupOpen,
			skillCandidates,
			skillSelectedIndex,
			handleSelectSkill,
			activePromptSuggestion,
			completeShellDraft,
			isStreaming,
			onInterrupt,
			value,
			promptHistory,
			historyIndex,
			setComposerValue,
			startDictation,
		],
	);

	const handleChange = useCallback(
		(e: React.ChangeEvent<HTMLTextAreaElement>) => {
			const newValue = e.target.value;
			setValue(newValue);
			setHistoryIndex(null);
			setSlashPopupDismissed(false);
			setSelectedIndex(0);

			// Detect mention trigger
			const cursorPos = e.target.selectionStart ?? newValue.length;
			const trigger = findMentionTrigger(newValue, cursorPos);
			if (trigger) {
				setMentionTrigger(trigger);
				setMentionDismissed(false);
				setSkillTrigger(null);
			} else {
				setMentionTrigger(null);
			}
			const nextSkillTrigger = findSkillTrigger(newValue, cursorPos);
			if (nextSkillTrigger) {
				setSkillTrigger(nextSkillTrigger);
				setSkillDismissed(false);
				setMentionTrigger(null);
			} else {
				setSkillTrigger(null);
			}

			const el = e.target;
			el.style.height = "auto";
			el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
		},
		[],
	);

	const handleHistorySearchKeyDown = useCallback(
		(e: React.KeyboardEvent<HTMLInputElement>) => {
			if (e.key.toLowerCase() === "s" && e.ctrlKey && !e.metaKey && !e.altKey) {
				e.preventDefault();
				cycleHistorySearchScope();
				return;
			}
			if (e.key === "Escape") {
				e.preventDefault();
				closeHistorySearch();
				return;
			}
			if (e.key === "Enter") {
				e.preventDefault();
				acceptHistorySearchMatch();
				return;
			}
			if (e.key === "ArrowDown") {
				e.preventDefault();
				setHistorySearchSelectedIndex((current) =>
					historySearchMatches.length === 0
						? 0
						: (current + 1) % historySearchMatches.length,
				);
				return;
			}
			if (
				e.key === "ArrowUp" ||
				(e.key.toLowerCase() === "r" && e.ctrlKey && !e.metaKey && !e.altKey)
			) {
				e.preventDefault();
				setHistorySearchSelectedIndex((current) =>
					historySearchMatches.length === 0
						? 0
						: (current - 1 + historySearchMatches.length) %
							historySearchMatches.length,
				);
			}
		},
		[
			acceptHistorySearchMatch,
			closeHistorySearch,
			cycleHistorySearchScope,
			historySearchMatches.length,
		],
	);

	const handlePaste = useCallback(
		async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
			const items = e.clipboardData?.items;
			if (!items) return;
			const imageFiles: File[] = [];
			for (const item of items) {
				if (item.type.startsWith("image/")) {
					const file = item.getAsFile();
					if (file) imageFiles.push(file);
				}
			}
			if (imageFiles.length > 0) {
				e.preventDefault();
				addImages(imageFiles);
				return;
			}
			const pastedText = e.clipboardData.getData?.("text/plain") ?? "";
			if (!pastedText) return;

			e.preventDefault();
			const el = e.currentTarget;
			const start = el.selectionStart ?? el.value.length;
			const end = el.selectionEnd ?? el.value.length;
			const nextIndex = pastedTextIdCounterRef.current + 1;
			try {
				const block = await invoke<PastedTextBlock | null>(
					"prepare_pasted_text_block",
					{
						index: nextIndex,
						content: pastedText,
					},
				);
				const insertion = block?.placeholder ?? pastedText;
				if (block) {
					pastedTextIdCounterRef.current = block.id;
					setPastedTextBlocks((current) => [...current, block]);
				}
				setHistoryIndex(null);
				setSlashPopupDismissed(false);
				setSelectedIndex(0);
				setComposerValueWithCaret(
					`${el.value.slice(0, start)}${insertion}${el.value.slice(end)}`,
					start + insertion.length,
				);
			} catch (err) {
				console.error("Failed to prepare pasted text block:", err);
				setComposerValueWithCaret(
					`${el.value.slice(0, start)}${pastedText}${el.value.slice(end)}`,
					start + pastedText.length,
				);
			}
		},
		[addImages, setComposerValueWithCaret],
	);

	const canSend = value.trim().length > 0 || attachedImages.length > 0;
	const streamingSubmitLabel =
		currentBackendId === "codex" ? "Steer active turn" : "Queue message";
	const submitLabel = isStreaming ? streamingSubmitLabel : "Send message";
	const inputRef = useRef<HTMLDivElement>(null);

	return (
		<>
			<div
				ref={inputRef}
				data-testid="message-input"
				className="relative mx-3 my-2 border rounded-lg focus-within:ring-1 focus-within:ring-ring"
			>
				{attachedImages.length > 0 && (
					<div
						data-testid="image-preview-list"
						className="flex flex-wrap gap-2 px-3 pt-2"
					>
						{attachedImages.map((img) => (
							<div
								key={img.id}
								className="relative group"
								data-testid="image-preview-item"
							>
								<img
									src={img.previewUrl}
									alt="Attached"
									className="h-16 w-16 object-cover rounded-md border"
								/>
								<button
									type="button"
									onClick={() => removeImage(img.id)}
									className="absolute -top-1.5 -right-1.5 bg-destructive text-destructive-foreground rounded-full p-0.5 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring transition-opacity"
									aria-label="Remove image"
									data-testid="remove-image-button"
								>
									<X className="size-3" />
								</button>
							</div>
						))}
					</div>
				)}
				{historySearchOpen && (
					<div
						className="border-b px-2 py-2"
						data-testid="prompt-history-search"
					>
						<div className="flex items-center gap-1 rounded border bg-background px-2 py-1">
							<Search className="size-3.5 shrink-0 text-muted-foreground" />
							<input
								ref={historySearchInputRef}
								type="search"
								value={historySearchQuery}
								onChange={(event) => {
									setHistorySearchQuery(event.target.value);
									setHistorySearchSelectedIndex(0);
								}}
								onKeyDown={handleHistorySearchKeyDown}
								placeholder="Search prompt history"
								aria-label="Search prompt history"
								className="min-w-0 flex-1 bg-transparent text-sm outline-none"
							/>
							<button
								type="button"
								className="inline-flex h-6 shrink-0 items-center rounded border border-border px-1.5 text-[11px] uppercase tracking-normal text-muted-foreground hover:bg-muted hover:text-foreground"
								aria-label="Cycle prompt history search scope"
								onClick={cycleHistorySearchScope}
							>
								{historySearchScope}
							</button>
							<span className="w-14 shrink-0 text-right text-xs text-muted-foreground tabular-nums">
								{historySearchMatches.length === 0
									? "0/0"
									: `${historySearchSelectedIndex + 1}/${historySearchMatches.length}`}
							</span>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
								aria-label="Previous prompt history match"
								disabled={historySearchMatches.length === 0}
								onClick={() =>
									setHistorySearchSelectedIndex((current) =>
										historySearchMatches.length === 0
											? 0
											: (current - 1 + historySearchMatches.length) %
												historySearchMatches.length,
									)
								}
							>
								<ChevronUp className="size-3.5" />
							</button>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted disabled:cursor-not-allowed disabled:opacity-50"
								aria-label="Next prompt history match"
								disabled={historySearchMatches.length === 0}
								onClick={() =>
									setHistorySearchSelectedIndex((current) =>
										historySearchMatches.length === 0
											? 0
											: (current + 1) % historySearchMatches.length,
									)
								}
							>
								<ChevronDown className="size-3.5" />
							</button>
							<button
								type="button"
								className="inline-flex size-6 shrink-0 items-center justify-center rounded hover:bg-muted"
								aria-label="Close prompt history search"
								onClick={closeHistorySearch}
							>
								<X className="size-3.5" />
							</button>
						</div>
						{historySearchMatches[historySearchSelectedIndex] && (
							<button
								type="button"
								className="mt-1 flex w-full min-w-0 items-center gap-2 rounded px-2 py-1 text-left text-xs text-muted-foreground hover:bg-muted hover:text-foreground"
								onClick={acceptHistorySearchMatch}
							>
								<span className="shrink-0 uppercase tracking-normal">
									{historySearchMatches[historySearchSelectedIndex].scope}
								</span>
								<span className="min-w-0 truncate">
									{historySearchMatches[historySearchSelectedIndex].text}
								</span>
							</button>
						)}
					</div>
				)}
				<textarea
					ref={textareaRef}
					value={value}
					onChange={handleChange}
					onKeyDown={handleKeyDown}
					onPaste={handlePaste}
					placeholder={activePromptSuggestion ?? "Send a message..."}
					rows={1}
					className="w-full resize-none bg-transparent border-none px-3 pt-3 pb-1 text-sm focus:outline-none min-h-[36px] max-h-[200px]"
				/>
				{activeSlashArgumentHelp && (
					<div
						className="border-t px-3 py-1.5 text-xs text-muted-foreground"
						data-testid="slash-argument-help"
					>
						<span className="font-medium text-foreground">
							/{activeSlashArgumentHelp.name}
						</span>{" "}
						<span>{activeSlashArgumentHelp.argumentHint}</span>
						{activeSlashArgumentHelp.description ? (
							<span className="ml-2">
								{activeSlashArgumentHelp.description}
							</span>
						) : null}
					</div>
				)}
				<div className="flex items-center justify-between px-2 pb-2">
					<div className="flex items-center gap-1">
						<BackendSelector
							backends={backends}
							selectedBackendId={currentBackendId}
							onBackendChange={onBackendChange}
							disabled={backendDisabled}
						/>
						<ModeSelector
							mode={mode}
							onModeChange={onModeChange}
							disabled={false}
						/>
						<ModelSelector
							models={models}
							currentModelId={currentModelId}
							onModelChange={onModelChange}
							disabled={false}
						/>
						<Button
							type="button"
							size="xs"
							variant="ghost"
							className="h-6 px-1.5"
							onClick={openPromptEditor}
							aria-label="Open prompt editor"
							title="Open prompt editor"
						>
							<Maximize2 className="size-3" />
						</Button>
						<Button
							type="button"
							size="xs"
							variant={dictationListening ? "secondary" : "ghost"}
							className="h-6 px-1.5"
							onClick={toggleDictation}
							disabled={!dictationPresentation.enabled && !dictationListening}
							aria-label={dictationPresentation.label}
							title={dictationPresentation.title}
						>
							<Mic className="size-3" />
						</Button>
					</div>
					<div className="flex items-center gap-2">
						{dictationPresentation.status && (
							<span
								className={
									dictationListening
										? "text-xs text-primary"
										: "text-xs text-muted-foreground"
								}
								role="status"
							>
								{dictationPresentation.status}
							</span>
						)}
						{isStreaming && canSend && (
							<span className="text-xs text-muted-foreground">
								{currentBackendId === "codex"
									? "Steer active turn"
									: "Queue follow-up"}
							</span>
						)}
						{isStreaming && (
							<Button
								size="icon"
								variant="destructive"
								className="h-7 w-7 shrink-0"
								onClick={onInterrupt}
								aria-label="Interrupt agent"
								title="Interrupt agent"
							>
								<Square className="size-3.5" />
							</Button>
						)}
						{(!isStreaming || canSend) && (
							<Button
								size="icon"
								className="h-7 w-7 shrink-0"
								onClick={handleSubmit}
								disabled={!canSend}
								aria-label={submitLabel}
								title={submitLabel}
							>
								<ArrowUp className="size-3.5" />
							</Button>
						)}
					</div>
				</div>
			</div>
			<SlashCommandPopup
				open={popupOpen}
				commands={filteredCommands}
				selectedIndex={selectedIndex}
				onSelect={handleSelectCommand}
				onClose={() => setSlashPopupDismissed(true)}
				anchorRef={inputRef}
			/>
			<MentionPopup
				open={mentionPopupOpen}
				files={mentionFiles}
				selectedIndex={mentionSelectedIndex}
				onSelect={handleSelectMention}
				onClose={() => setMentionDismissed(true)}
				anchorRef={inputRef}
			/>
			<SkillPopup
				open={skillPopupOpen}
				skills={skillCandidates}
				selectedIndex={skillSelectedIndex}
				onSelect={handleSelectSkill}
				onClose={() => setSkillDismissed(true)}
				anchorRef={inputRef}
			/>
			<Dialog
				open={promptEditorOpen}
				onOpenChange={handlePromptEditorOpenChange}
			>
				<DialogContent className="max-h-[90vh] sm:max-w-4xl">
					<DialogHeader>
						<DialogTitle>Prompt editor</DialogTitle>
						<DialogDescription className="sr-only">
							Edit the current prompt in a larger text area.
						</DialogDescription>
					</DialogHeader>
					<textarea
						ref={promptEditorRef}
						value={promptEditorValue}
						onChange={(event) => setPromptEditorValue(event.target.value)}
						className="min-h-[55vh] w-full resize-none rounded border bg-background p-3 font-mono text-sm outline-none focus:ring-1 focus:ring-ring"
						aria-label="Prompt editor text"
					/>
					{promptEditorError && (
						<div className="rounded border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
							{promptEditorError}
						</div>
					)}
					<DialogFooter>
						<div className="mr-auto flex items-center gap-2">
							<Button
								type="button"
								variant="outline"
								onClick={openExternalPromptEditor}
								disabled={isOpeningExternalEditor || isImportingExternalEditor}
							>
								<ExternalLink className="size-3.5" />
								{isOpeningExternalEditor ? "Opening" : "Open external"}
							</Button>
							<Button
								type="button"
								variant="outline"
								onClick={importExternalPromptEdits}
								disabled={!externalPromptDraftId || isImportingExternalEditor}
							>
								<FileInput className="size-3.5" />
								{isImportingExternalEditor ? "Importing" : "Import edits"}
							</Button>
						</div>
						<Button
							type="button"
							variant="outline"
							onClick={() => handlePromptEditorOpenChange(false)}
						>
							Cancel
						</Button>
						<Button type="button" onClick={applyPromptEditorValue}>
							Apply
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
}
