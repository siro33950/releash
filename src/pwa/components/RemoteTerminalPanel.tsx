import { useCallback, useEffect, useRef, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "../hooks/useMessageBus";
import { useRemoteTerminal } from "../hooks/useRemoteTerminal";

interface RemoteTerminalPanelProps {
	ptyId: number;
	ptyCols: number;
	send: (msg: WsMessage) => void;
	subscribe: Subscribe;
	visible: boolean;
}

type KeyDef =
	| { label: string; key: string }
	| { label: string; modifier: "ctrl" };

const BAR_KEYS: KeyDef[] = [
	{ label: "Ctrl", modifier: "ctrl" },
	{ label: "Enter", key: "\r" },
	{ label: "Del", key: "\x7f" },
	{ label: "Esc", key: "\x1b" },
	{ label: "Tab", key: "\t" },
	{ label: "\u2191", key: "\x1b[A" },
	{ label: "\u2193", key: "\x1b[B" },
	{ label: "\u2190", key: "\x1b[D" },
	{ label: "\u2192", key: "\x1b[C" },
];

const MENU_SHORTCUTS = [
	{ label: "C-c", key: "\x03" },
	{ label: "C-d", key: "\x04" },
	{ label: "C-z", key: "\x1a" },
	{ label: "C-l", key: "\x0c" },
	{ label: "S+Tab", key: "\x1b[Z" },
	{ label: "/", key: "/" },
	{ label: "@", key: "@" },
	{ label: "!", key: "!" },
	{ label: "|", key: "|" },
	{ label: "~", key: "~" },
	{ label: "y", key: "y" },
	{ label: "n", key: "n" },
];

type ModifierState = null | { type: "ctrl"; locked: boolean };

const DOUBLE_TAP_MS = 300;

export function RemoteTerminalPanel({
	ptyId,
	ptyCols,
	send,
	subscribe,
	visible,
}: RemoteTerminalPanelProps) {
	const rootRef = useRef<HTMLDivElement>(null);
	const containerRef = useRef<HTMLDivElement>(null);
	const inputRef = useRef<HTMLTextAreaElement>(null);
	const [modifier, setModifier] = useState<ModifierState>(null);
	const [inputValue, setInputValue] = useState("");
	const [menuOpen, setMenuOpen] = useState(false);
	const lastCtrlTapRef = useRef(0);

	useRemoteTerminal({ containerRef, ptyId, ptyCols, send, subscribe, visible });

	useEffect(() => {
		const viewport = window.visualViewport;
		const root = rootRef.current;
		if (!viewport || !root) return;

		const initialHeight = viewport.height;

		const handleResize = () => {
			if (viewport.height < initialHeight * 0.9) {
				const rect = root.getBoundingClientRect();
				const topInViewport = rect.top - viewport.offsetTop;
				const available = viewport.height - topInViewport;
				root.style.height = `${Math.max(0, available)}px`;
			} else {
				root.style.height = "";
			}
		};

		viewport.addEventListener("resize", handleResize);
		viewport.addEventListener("scroll", handleResize);
		return () => {
			viewport.removeEventListener("resize", handleResize);
			viewport.removeEventListener("scroll", handleResize);
			root.style.height = "";
		};
	}, []);

	// メニュー外タップで閉じる
	useEffect(() => {
		if (!menuOpen) return;
		const handlePointerDown = () => setMenuOpen(false);
		document.addEventListener("pointerdown", handlePointerDown);
		return () => document.removeEventListener("pointerdown", handlePointerDown);
	}, [menuOpen]);

	const sendPtyInput = useCallback(
		(data: string) => {
			send({ type: "pty_input", payload: { pty_id: ptyId, data } });
		},
		[send, ptyId],
	);

	const applyCtrlToChar = useCallback((char: string): string => {
		const code = char.toUpperCase().charCodeAt(0) - 64;
		if (code >= 0 && code <= 31) {
			return String.fromCharCode(code);
		}
		return char;
	}, []);

	const consumeModifier = useCallback((): boolean => {
		if (!modifier) return false;
		if (!modifier.locked) {
			setModifier(null);
		}
		return true;
	}, [modifier]);

	const handleSubmit = useCallback(() => {
		if (inputValue.length === 0) return;
		let data = inputValue;
		if (consumeModifier()) {
			data = applyCtrlToChar(data[0]) + data.slice(1);
		}
		sendPtyInput(data);
		setInputValue("");
	}, [inputValue, consumeModifier, sendPtyInput, applyCtrlToChar]);

	const handleCtrlTap = useCallback(() => {
		const now = Date.now();
		const elapsed = now - lastCtrlTapRef.current;
		lastCtrlTapRef.current = now;

		setModifier((prev) => {
			if (prev === null) {
				// 1回タップ → Sticky
				return { type: "ctrl", locked: false };
			}
			if (!prev.locked && elapsed < DOUBLE_TAP_MS) {
				// ダブルタップ → Lock
				return { type: "ctrl", locked: true };
			}
			// Lock中タップ or Sticky中再タップ → 解除
			return null;
		});
	}, []);

	const handleBarKey = useCallback(
		(def: KeyDef) => {
			if ("modifier" in def) {
				handleCtrlTap();
				return;
			}
			let key = def.key;
			if (consumeModifier() && key.length === 1) {
				key = applyCtrlToChar(key);
			}
			sendPtyInput(key);
			inputRef.current?.focus();
		},
		[handleCtrlTap, consumeModifier, sendPtyInput, applyCtrlToChar],
	);

	const handleShortcut = useCallback(
		(key: string) => {
			sendPtyInput(key);
			setMenuOpen(false);
			inputRef.current?.focus();
		},
		[sendPtyInput],
	);

	const ctrlButtonClass = modifier
		? modifier.locked
			? "bg-blue-600 text-white ring-2 ring-blue-400"
			: "bg-blue-600 text-white"
		: "bg-[#333] text-[#ccc] active:bg-[#555]";

	return (
		<div ref={rootRef} className="flex flex-col h-full">
			<div
				ref={containerRef}
				className="flex-1 overflow-hidden bg-[#1a1a1a]"
				style={{ minHeight: 0 }}
			/>

			<div className="flex items-center px-2 py-1 bg-[#252525] border-t border-[#333] shrink-0">
				<div className="flex gap-1 overflow-x-auto flex-1">
					{BAR_KEYS.map((def) => {
						const isCtrl = "modifier" in def;
						return (
							<button
								key={def.label}
								type="button"
								className={`px-2 py-1 text-xs rounded shrink-0 ${
									isCtrl
										? ctrlButtonClass
										: "bg-[#333] text-[#ccc] active:bg-[#555]"
								}`}
								onPointerDown={(e) => {
									e.preventDefault();
									handleBarKey(def);
								}}
							>
								{def.label}
							</button>
						);
					})}
				</div>

				<div className="ml-1 relative">
					<button
						type="button"
						className={`px-2 py-1 text-xs rounded shrink-0 ${
							menuOpen
								? "bg-blue-600 text-white"
								: "bg-[#333] text-[#ccc] active:bg-[#555]"
						}`}
						onPointerDown={(e) => {
							e.preventDefault();
							e.stopPropagation();
							setMenuOpen((prev) => !prev);
						}}
					>
						⋯
					</button>

					{menuOpen && (
						<div
							className="absolute bottom-full right-0 mb-1 bg-[#2d2d2d] border border-[#555] rounded p-1 grid grid-cols-4 gap-1 z-50"
							style={{ minWidth: "180px" }}
							onPointerDown={(e) => e.stopPropagation()}
						>
							{MENU_SHORTCUTS.map((sc) => (
								<button
									key={sc.label}
									type="button"
									className="px-2 py-1.5 text-xs bg-[#333] text-[#ccc] rounded active:bg-[#555] whitespace-nowrap"
									onPointerDown={(e) => {
										e.preventDefault();
										handleShortcut(sc.key);
									}}
								>
									{sc.label}
								</button>
							))}
						</div>
					)}
				</div>
			</div>

			<div className="flex gap-2 px-2 py-2 bg-[#1e1e1e] border-t border-[#333] shrink-0">
				<textarea
					ref={inputRef}
					value={inputValue}
					onChange={(e) => setInputValue(e.target.value)}
					rows={1}
					autoComplete="off"
					autoCorrect="off"
					autoCapitalize="off"
					spellCheck={false}
					className="flex-1 px-3 py-2 bg-[#2d2d2d] text-[#e0e0e0] border border-[#444] rounded text-sm outline-none focus:border-blue-500 resize-none max-h-24 leading-5"
					placeholder="コマンドを入力..."
				/>
				<button
					type="button"
					className="px-4 py-2 bg-blue-600 text-white rounded text-sm active:bg-blue-700 shrink-0"
					onClick={handleSubmit}
				>
					送信
				</button>
			</div>
		</div>
	);
}
