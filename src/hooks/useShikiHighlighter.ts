import { useEffect, useRef, useState } from "react";
import type { ThemedToken } from "shiki/core";
import type { TokenizeRequest, TokenizeResponse } from "@/workers/shiki.worker";

export type { ThemedToken };

export interface TokenizedLine {
	tokens: ThemedToken[];
}

let worker: Worker | null = null;
let nextRequestId = 0;
const pendingCallbacks = new Map<number, (lines: TokenizedLine[]) => void>();

function getWorker(): Worker {
	if (worker) return worker;

	worker = new Worker(new URL("../workers/shiki.worker.ts", import.meta.url), {
		type: "module",
	});

	worker.onmessage = (e: MessageEvent<TokenizeResponse>) => {
		const { id, lines } = e.data;
		const cb = pendingCallbacks.get(id);
		if (cb) {
			pendingCallbacks.delete(id);
			cb(lines as TokenizedLine[]);
		}
	};

	return worker;
}

function tokenizeInWorker(
	code: string,
	language: string,
): { id: number; cancel: () => void } {
	const id = ++nextRequestId;
	const w = getWorker();

	w.postMessage({ id, code, language } satisfies TokenizeRequest);

	return {
		id,
		cancel: () => {
			pendingCallbacks.delete(id);
		},
	};
}

export function preloadHighlighter(): void {
	getWorker();
}

export function useShikiHighlighter(
	code: string,
	language: string,
): TokenizedLine[] | null {
	const [lines, setLines] = useState<TokenizedLine[] | null>(null);
	const requestRef = useRef<{ cancel: () => void } | null>(null);

	useEffect(() => {
		setLines(null);

		if (requestRef.current) {
			requestRef.current.cancel();
		}

		if (code === "") {
			setLines([]);
			return;
		}

		const req = tokenizeInWorker(code, language);
		requestRef.current = req;

		pendingCallbacks.set(req.id, (result) => {
			requestRef.current = null;
			setLines(result);
		});

		return () => {
			req.cancel();
			requestRef.current = null;
		};
	}, [code, language]);

	return lines;
}
