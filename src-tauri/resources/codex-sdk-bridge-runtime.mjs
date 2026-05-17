import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
	codexEventToBridgeMessages,
	createThreadOptions,
} from "./codex-sdk-bridge-utils.mjs";

export class CodexBridgeRuntime {
	constructor({
		codex,
		codexFactory,
		emit,
		writeError = (text) => process.stderr.write(text),
		exit = (code) => process.exit(code),
		cwd = process.cwd(),
		tmpDir = os.tmpdir(),
		fsApi = fs,
		pathApi = path,
		now = () => Date.now(),
		random = () => Math.random(),
	}) {
		this.codex = codex;
		this.codexFactory = codexFactory;
		this.emit = emit;
		this.writeError = writeError;
		this.exit = exit;
		this.defaultCwd = cwd;
		this.tmpDir = tmpDir;
		this.fs = fsApi;
		this.path = pathApi;
		this.now = now;
		this.random = random;

		this.messageQueue = [];
		this.tempImageDirs = new Set();
		this.currentThread = null;
		this.currentThreadId = null;
		this.currentModelId = null;
		this.currentPermissionMode = "acceptEdits";
		this.currentCwd = cwd;
		this.currentAbortController = null;
		this.messageResolve = null;
		this.closed = false;
		this.exitCode = 0;
		this.activeTurn = false;
		this.initialResumeThreadId = null;
		this.stdinBuffer = "";
		this.completion = null;
	}

	handleInputChunk(chunk) {
		this.stdinBuffer += chunk;
		const lines = this.stdinBuffer.split("\n");
		this.stdinBuffer = lines.pop();
		for (const line of lines) {
			if (!line.trim()) continue;
			try {
				this.handleCommand(JSON.parse(line));
			} catch (e) {
				this.writeError(
					`codex bridge: stdin parse error: ${e instanceof Error ? e.message : String(e)}\n`,
				);
			}
		}
	}

	handleCommand(cmd) {
		switch (cmd.type) {
			case "init":
				this.handleInit(cmd);
				break;
			case "message":
				this.enqueueMessage(cmd);
				break;
			case "interrupt":
				void this.currentThread?.interrupt?.();
				this.currentAbortController?.abort();
				break;
			case "setMode":
				this.currentPermissionMode = cmd.permissionMode || "acceptEdits";
				this.recreateThreadForNextTurn();
				break;
			case "setModel":
				this.currentModelId = cmd.modelId || null;
				this.recreateThreadForNextTurn();
				break;
			case "permission_response":
				this.writeError(
					`codex bridge: permission responses are not supported by @openai/codex-sdk streamed events (${cmd.request_id ?? "unknown request"})\n`,
				);
				break;
			case "close":
				this.close();
				break;
		}
	}

	handleInit(cmd) {
		if (!this.codex && this.codexFactory) {
			this.codex = this.codexFactory({
				cliPath: cmd.codexCliPath || "codex",
			});
		}
		if (!this.codex) {
			throw new Error("Codex SDK is not initialized");
		}
		this.currentCwd = cmd.cwd || this.defaultCwd;
		this.currentPermissionMode = cmd.permissionMode || "acceptEdits";
		this.currentModelId = cmd.model || null;
		this.initialResumeThreadId = cmd.sessionId || null;
		this.currentThreadId = this.initialResumeThreadId;
		this.currentThread = this.createThread();

		// Codex のモデル一覧は起動時 CLI 同期（`codex debug models`）で config.toml に
		// 反映するため、bridge からは supported_models を emit しない（ハードコード一覧の撤去）。
		this.emit({
			type: "session_ready",
			session_id: this.currentThreadId,
			initialized: true,
		});
		this.completion = this.runMessageLoop().catch((e) => {
			this.emit({
				type: "error",
				message: e instanceof Error ? e.message : String(e),
				clear_session_id: Boolean(this.initialResumeThreadId),
			});
			this.exit(1);
		});
	}

	close() {
		this.closed = true;
		this.currentAbortController?.abort();
		void this.codex?.close?.();
		if (this.messageResolve) {
			this.messageResolve(null);
			this.messageResolve = null;
		}
	}

	buildThreadOptions() {
		return createThreadOptions({
			cwd: this.currentCwd,
			modelId: this.currentModelId,
			permissionMode: this.currentPermissionMode,
		});
	}

	createThread({ forceNew = false } = {}) {
		if (!forceNew && this.currentThreadId) {
			return this.codex.resumeThread(this.currentThreadId, this.buildThreadOptions());
		}
		return this.codex.startThread(this.buildThreadOptions());
	}

	recreateThreadForNextTurn() {
		if (this.activeTurn || this.closed) return;
		this.currentThread = this.createThread();
	}

	enqueueMessage(cmd) {
		const message = { prompt: cmd.prompt || "", images: cmd.images || [] };
		if (this.messageResolve) {
			this.messageResolve(message);
			this.messageResolve = null;
		} else {
			this.messageQueue.push(message);
		}
	}

	async nextMessage() {
		if (this.messageQueue.length > 0) return this.messageQueue.shift();
		return new Promise((resolve) => {
			this.messageResolve = resolve;
		});
	}

	async runMessageLoop() {
		while (!this.closed) {
			const message = await this.nextMessage();
			if (message === null || this.closed) break;
			await this.runTurn(message);
		}
		await this.cleanupTempImages();
		this.exit(this.exitCode);
	}

	async runTurn(message) {
		this.activeTurn = true;
		this.currentAbortController = new AbortController();
		const state = {
			threadId: this.currentThreadId,
			itemText: new Map(),
			clearSessionIdOnFailure: Boolean(this.initialResumeThreadId),
			eventsSeen: false,
		};

		try {
			await this.runTurnOnce(message, state);
			this.initialResumeThreadId = null;
		} catch (e) {
			if (this.shouldFallbackFromResumeFailure(state, e)) {
				this.emit({
					type: "session_cleared",
					session_id: this.initialResumeThreadId,
					reason: "resume_failed",
				});
				this.currentThreadId = null;
				this.initialResumeThreadId = null;
				state.threadId = null;
				state.clearSessionIdOnFailure = false;
				state.eventsSeen = false;
				this.currentThread = this.createThread({ forceNew: true });

				try {
					await this.runTurnOnce(message, state);
				} catch (retryError) {
					this.handleTurnError(retryError);
				}
			} else {
				this.handleTurnError(e);
			}
		} finally {
			this.activeTurn = false;
			this.currentAbortController = null;
			await this.cleanupTempImages();
			if (!this.closed) {
				this.recreateThreadForNextTurn();
			}
		}
	}

	async runTurnOnce(message, state) {
		const input = await this.buildInput(message);
		const { events } = await this.currentThread.runStreamed(input, {
			signal: this.currentAbortController.signal,
		});
		for await (const event of events) {
			state.eventsSeen = true;
			const messages = codexEventToBridgeMessages(event, state);
			this.emitMany(messages);
			if (state.threadId && state.threadId !== this.currentThreadId) {
				this.currentThreadId = state.threadId;
				this.initialResumeThreadId = null;
			}
			if (messages.some(isFailedTurnComplete)) {
				this.closed = true;
				this.exitCode = 1;
				break;
			}
		}
	}

	shouldFallbackFromResumeFailure(state, error) {
		return (
			Boolean(this.initialResumeThreadId) &&
			!state.eventsSeen &&
			!this.currentAbortController.signal.aborted &&
			isResumeFailureError(error)
		);
	}

	handleTurnError(error) {
		if (this.currentAbortController.signal.aborted) {
			this.emit({
				type: "turn_complete",
				session_id: this.currentThreadId || null,
				exit_code: 0,
			});
			return;
		}

		this.closed = true;
		this.exitCode = 1;
		this.emit({
			type: "error",
			message: error instanceof Error ? error.message : String(error),
			clear_session_id: Boolean(this.initialResumeThreadId),
		});
		this.emit({
			type: "turn_complete",
			session_id: this.currentThreadId || null,
			exit_code: 1,
		});
	}

	emitMany(messages) {
		for (const message of messages) {
			this.emit(message);
		}
	}

	async buildInput(message) {
		if (!message.images.length) return message.prompt;

		const entries = [];
		if (message.prompt) {
			entries.push({ type: "text", text: message.prompt });
		}
		for (const image of message.images) {
			entries.push({
				type: "local_image",
				path: await this.writeTempImage(image),
			});
		}
		return entries;
	}

	async writeTempImage(image) {
		const ext = extensionForMediaType(image.mediaType || image.media_type);
		const dir = await this.fs.mkdtemp(
			this.path.join(this.tmpDir, "releash-codex-"),
		);
		await this.fs.chmod?.(dir, 0o700);
		this.tempImageDirs.add(dir);
		const file = this.path.join(
			dir,
			`${this.now()}-${this.random().toString(36).slice(2)}.${ext}`,
		);
		await this.fs.writeFile(file, Buffer.from(image.data, "base64"), {
			mode: 0o600,
		});
		return file;
	}

	async cleanupTempImages() {
		const dirs = Array.from(this.tempImageDirs);
		this.tempImageDirs.clear();
		await Promise.all(
			dirs.map(async (dir) => {
				try {
					if (this.fs.rm) {
						await this.fs.rm(dir, { recursive: true, force: true });
					} else {
						await this.fs.rmdir(dir, { recursive: true });
					}
				} catch (e) {
					this.writeError(
						`codex bridge: failed to remove temp image dir ${dir}: ${e instanceof Error ? e.message : String(e)}\n`,
					);
				}
			}),
		);
	}
}

function isFailedTurnComplete(message) {
	return message.type === "turn_complete" && message.exit_code !== 0;
}

function isResumeFailureError(error) {
	const message = error instanceof Error ? error.message : String(error);
	return /(?:resume|thread|session|conversation).*(?:not found|invalid|missing|expired)|(?:not found|invalid|missing|expired).*(?:resume|thread|session|conversation)|no conversation/i.test(
		message,
	);
}

export function extensionForMediaType(mediaType) {
	switch (mediaType) {
		case "image/jpeg":
			return "jpg";
		case "image/gif":
			return "gif";
		case "image/webp":
			return "webp";
		default:
			return "png";
	}
}

export function startCodexBridge({
	codex,
	codexFactory,
	stdin = process.stdin,
	stdout = process.stdout,
	stderr = process.stderr,
	exit = (code) => process.exit(code),
} = {}) {
	const runtime = new CodexBridgeRuntime({
		codex,
		codexFactory,
		emit: (obj) => stdout.write(`${JSON.stringify(obj)}\n`),
		writeError: (text) => stderr.write(text),
		exit,
	});

	stdin.setEncoding("utf8");
	stdin.on("data", (chunk) => {
		runtime.handleInputChunk(chunk);
	});

	return runtime;
}
