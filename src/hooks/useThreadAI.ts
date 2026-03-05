import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { type AppSettings, buildThreadCommand } from "@/types/settings";

export type ThreadAITaskStatus = "running" | "completed" | "error";

export interface ThreadAITask {
	threadId: string;
	filePath: string;
	lineInfo: string;
	mode: "ask" | "summarize";
	status: ThreadAITaskStatus;
	ptyId: number | null;
	output: string;
}

interface ThreadAiPrompt {
	prompt: string;
	thread_id: string;
	file_path: string;
}

interface UseThreadAIOptions {
	onCompleted?: (threadId: string, output: string) => void;
}

export function useThreadAI(
	worktreePath: string | null,
	settings: AppSettings,
	options?: UseThreadAIOptions,
) {
	const [taskMap, setTaskMap] = useState<Map<string, ThreadAITask>>(new Map());
	const taskMapRef = useRef<Map<string, ThreadAITask>>(new Map());
	const ptyThreadMapRef = useRef<Map<number, string>>(new Map());
	const pendingOutputRef = useRef<Map<number, string>>(new Map());
	const pendingStatusRef = useRef<
		Map<number, { status: string; exit_code: number | null }>
	>(new Map());

	const worktreePathRef = useRef(worktreePath);
	worktreePathRef.current = worktreePath;
	const settingsRef = useRef(settings);
	settingsRef.current = settings;
	const onCompletedRef = useRef(options?.onCompleted);
	onCompletedRef.current = options?.onCompleted;

	const syncState = useCallback(() => {
		setTaskMap(new Map(taskMapRef.current));
	}, []);

	const handlePtyFinished = useCallback(
		(ptyId: number, status: string) => {
			const threadId = ptyThreadMapRef.current.get(ptyId);
			if (!threadId) return;

			ptyThreadMapRef.current.delete(ptyId);
			pendingStatusRef.current.delete(ptyId);
			pendingOutputRef.current.delete(ptyId);

			const task = taskMapRef.current.get(threadId);
			if (!task) return;

			const isError =
				status === "error" || status === "timeout" || status === "cancelled";
			const newTask: ThreadAITask = {
				...task,
				status: isError ? "error" : "completed",
			};
			taskMapRef.current.set(threadId, newTask);
			syncState();

			if (!isError) {
				onCompletedRef.current?.(threadId, newTask.output);
			}
		},
		[syncState],
	);

	const handlePtyFinishedRef = useRef(handlePtyFinished);
	handlePtyFinishedRef.current = handlePtyFinished;

	// Cancel all running PTYs on unmount
	useEffect(() => {
		return () => {
			for (const ptyId of ptyThreadMapRef.current.keys()) {
				invoke("cancel_oneshot_pty", { ptyId }).catch(() => {});
			}
		};
	}, []);

	// Listen for PTY output
	useEffect(() => {
		const unlisten = listen<{ pty_id: number; data: string }>(
			"pty-output",
			(event) => {
				const { pty_id, data } = event.payload;
				const threadId = ptyThreadMapRef.current.get(pty_id);

				if (threadId) {
					const task = taskMapRef.current.get(threadId);
					if (task) {
						const updated: ThreadAITask = {
							...task,
							output: task.output + data,
						};
						taskMapRef.current.set(threadId, updated);
						syncState();
					}
				} else {
					// Buffer output for PTY IDs we haven't matched yet
					const buf = pendingOutputRef.current.get(pty_id) ?? "";
					pendingOutputRef.current.set(pty_id, buf + data);
				}
			},
		);

		return () => {
			unlisten.then((f) => f());
		};
	}, [syncState]);

	// Listen for PTY status changes
	useEffect(() => {
		const unlisten = listen<{
			pty_id: number;
			status: string;
			exit_code: number | null;
		}>("oneshot-pty-status-changed", (event) => {
			const { pty_id, status } = event.payload;

			const isTerminal =
				status === "completed" ||
				status === "error" ||
				status === "timeout" ||
				status === "cancelled";
			if (!isTerminal) return;

			if (ptyThreadMapRef.current.has(pty_id)) {
				handlePtyFinishedRef.current(pty_id, status);
			} else {
				// Buffer it - might arrive before spawn returns
				pendingStatusRef.current.set(pty_id, {
					status,
					exit_code: event.payload.exit_code,
				});
			}
		});

		return () => {
			unlisten.then((f) => f());
		};
	}, []);

	const spawnTask = useCallback(
		async (threadId: string, mode: "ask" | "summarize", prNumber?: number) => {
			const wt = worktreePathRef.current;
			if (!wt) return;

			// If already running for this thread, do nothing
			const existing = taskMapRef.current.get(threadId);
			if (existing?.status === "running") return;

			// Set running state before any await to prevent duplicate spawns
			const placeholder: ThreadAITask = {
				threadId,
				filePath: "",
				lineInfo: "",
				mode,
				status: "running",
				ptyId: null,
				output: "",
			};
			taskMapRef.current.set(threadId, placeholder);
			syncState();

			try {
				const invokeCmd =
					mode === "ask"
						? "build_thread_ai_prompt"
						: "build_thread_summarize_prompt";
				const result = await invoke<ThreadAiPrompt>(invokeCmd, {
					worktreePath: wt,
					threadId,
					prNumber: prNumber ?? null,
				});

				const command = buildThreadCommand(settingsRef.current, result.prompt);
				if (!command) {
					const errorTask: ThreadAITask = {
						threadId,
						filePath: result.file_path,
						lineInfo: "",
						mode,
						status: "error",
						ptyId: null,
						output: "No AI agent configured",
					};
					taskMapRef.current.set(threadId, errorTask);
					syncState();
					return;
				}

				// Update placeholder with actual file path
				const current = taskMapRef.current.get(threadId);
				if (current) {
					const task: ThreadAITask = {
						...current,
						filePath: result.file_path,
					};
					taskMapRef.current.set(threadId, task);
					syncState();
				}

				const info = await invoke<{
					pty_id: number;
					session_key: string;
					status: string;
				}>("spawn_oneshot_pty", {
					command,
					worktreePath: wt,
					label: `thread-${mode}:${threadId}`,
					timeoutSecs: 120,
				});

				ptyThreadMapRef.current.set(info.pty_id, threadId);

				// Update task with ptyId
				const currentTask = taskMapRef.current.get(threadId);
				if (currentTask) {
					const updated: ThreadAITask = {
						...currentTask,
						ptyId: info.pty_id,
					};

					// Flush pending output
					const buffered = pendingOutputRef.current.get(info.pty_id);
					if (buffered) {
						updated.output = updated.output + buffered;
						pendingOutputRef.current.delete(info.pty_id);
					}

					taskMapRef.current.set(threadId, updated);

					// Flush pending status if process already finished
					const pendingStatus = pendingStatusRef.current.get(info.pty_id);
					if (pendingStatus) {
						pendingStatusRef.current.delete(info.pty_id);
						handlePtyFinishedRef.current(info.pty_id, pendingStatus.status);
					} else {
						syncState();
					}
				}
			} catch (err) {
				const errorTask: ThreadAITask = {
					threadId,
					filePath: "",
					lineInfo: "",
					mode,
					status: "error",
					ptyId: null,
					output: String(err),
				};
				taskMapRef.current.set(threadId, errorTask);
				syncState();
			}
		},
		[syncState],
	);

	const askAI = useCallback(
		(threadId: string, prNumber?: number) => {
			spawnTask(threadId, "ask", prNumber);
		},
		[spawnTask],
	);

	const summarizeForPr = useCallback(
		(threadId: string, prNumber?: number) => {
			spawnTask(threadId, "summarize", prNumber);
		},
		[spawnTask],
	);

	const cancelTask = useCallback(
		(threadId: string) => {
			const task = taskMapRef.current.get(threadId);
			if (task?.ptyId != null) {
				invoke("cancel_oneshot_pty", { ptyId: task.ptyId }).catch(() => {});
				ptyThreadMapRef.current.delete(task.ptyId);
			}
			const updated: ThreadAITask = {
				...(task ?? {
					threadId,
					filePath: "",
					lineInfo: "",
					mode: "ask" as const,
					ptyId: null,
					output: "",
				}),
				status: "error",
			};
			taskMapRef.current.set(threadId, updated);
			syncState();
		},
		[syncState],
	);

	const removeTask = useCallback(
		(threadId: string) => {
			const task = taskMapRef.current.get(threadId);
			if (task?.ptyId != null && task.status === "running") {
				invoke("cancel_oneshot_pty", { ptyId: task.ptyId }).catch(() => {});
				ptyThreadMapRef.current.delete(task.ptyId);
			}
			taskMapRef.current.delete(threadId);
			syncState();
		},
		[syncState],
	);

	return { taskMap, askAI, summarizeForPr, cancelTask, removeTask };
}
