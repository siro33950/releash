import { invoke } from "@tauri-apps/api/core";
import {
	forwardRef,
	useCallback,
	useImperativeHandle,
	useRef,
	useState,
} from "react";
import {
	TerminalPanel,
	type TerminalPanelHandle,
} from "@/components/panels/TerminalPanel";
import type { Theme } from "@/types/settings";

const MAX_TABS = 8;

interface Tab {
	id: string;
	label: string;
	ptyId: number | null;
}

export interface TerminalTabPanelHandle {
	writeToTerminal: (data: string) => void;
}

interface TerminalTabPanelProps {
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	agentType?: string;
	sessionKey?: string;
}

let tabIdCounter = 0;
function nextTabId() {
	tabIdCounter += 1;
	return `tab-${tabIdCounter}`;
}

export const TerminalTabPanel = forwardRef<
	TerminalTabPanelHandle,
	TerminalTabPanelProps
>(function TerminalTabPanel(
	{ cwd, theme, terminalStartupCommand, agentType, sessionKey },
	ref,
) {
	const [tabs, setTabs] = useState<Tab[]>(() => [
		{ id: nextTabId(), label: "Terminal 1", ptyId: null },
	]);
	const [activeTabId, setActiveTabId] = useState<string>(tabs[0].id);
	const terminalRefs = useRef<Map<string, TerminalPanelHandle>>(new Map());

	useImperativeHandle(
		ref,
		() => ({
			writeToTerminal: (data: string) => {
				const handle = terminalRefs.current.get(activeTabId);
				handle?.writeToTerminal(data);
			},
		}),
		[activeTabId],
	);

	const tabCounter = useRef(1);

	const addTab = useCallback(() => {
		setTabs((prev) => {
			if (prev.length >= MAX_TABS) return prev;
			tabCounter.current += 1;
			const num = tabCounter.current;
			const tab: Tab = {
				id: nextTabId(),
				label: `Terminal ${num}`,
				ptyId: null,
			};
			setActiveTabId(tab.id);
			return [...prev, tab];
		});
	}, []);

	const closeTab = useCallback(
		(tabId: string) => {
			setTabs((prev) => {
				if (prev.length <= 1) return prev;
				const tab = prev.find((t) => t.id === tabId);
				if (tab?.ptyId != null) {
					invoke("kill_pty", { ptyId: tab.ptyId }).catch(() => {});
				}
				const next = prev.filter((t) => t.id !== tabId);
				if (activeTabId === tabId) {
					setActiveTabId(next[0].id);
				}
				return next;
			});
		},
		[activeTabId],
	);

	const setTerminalRef = useCallback(
		(tabId: string) => (handle: TerminalPanelHandle | null) => {
			if (handle) {
				terminalRefs.current.set(tabId, handle);
			} else {
				terminalRefs.current.delete(tabId);
			}
		},
		[],
	);

	return (
		<div className="flex flex-col h-full">
			<div
				role="tablist"
				aria-label="ターミナルタブ"
				className="flex items-center gap-0.5 px-1 py-0.5 border-b border-border bg-card shrink-0 overflow-x-auto"
			>
				{tabs.map((tab) => (
					<div
						key={tab.id}
						className={`group flex items-center gap-1 px-2 py-0.5 text-xs rounded transition-colors shrink-0 cursor-pointer ${
							activeTabId === tab.id
								? "bg-primary text-primary-foreground"
								: "bg-secondary text-muted-foreground hover:bg-secondary/80"
						}`}
						role="tab"
						tabIndex={activeTabId === tab.id ? 0 : -1}
						aria-selected={activeTabId === tab.id}
						onClick={() => setActiveTabId(tab.id)}
						onKeyDown={(e) => {
							if (e.key === "Enter") setActiveTabId(tab.id);
							if (e.key === "ArrowRight") {
								e.preventDefault();
								const idx = tabs.findIndex((t) => t.id === tab.id);
								const next = tabs[idx + 1];
								if (next) setActiveTabId(next.id);
							}
							if (e.key === "ArrowLeft") {
								e.preventDefault();
								const idx = tabs.findIndex((t) => t.id === tab.id);
								const prev = tabs[idx - 1];
								if (prev) setActiveTabId(prev.id);
							}
						}}
					>
						<span>{tab.label}</span>
						{tabs.length > 1 && (
							<button
								type="button"
								className={`ml-0.5 rounded-sm hover:bg-black/20 inline-flex items-center ${
									activeTabId === tab.id
										? "opacity-80"
										: "opacity-0 group-hover:opacity-60"
								}`}
								onClick={(e) => {
									e.stopPropagation();
									closeTab(tab.id);
								}}
								aria-label={`Close ${tab.label}`}
							>
								&#x2715;
							</button>
						)}
					</div>
				))}
				{tabs.length < MAX_TABS && (
					<button
						type="button"
						className="px-1.5 py-0.5 text-xs text-muted-foreground hover:text-foreground hover:bg-secondary rounded transition-colors shrink-0"
						onClick={addTab}
						aria-label="Add terminal tab"
					>
						+
					</button>
				)}
			</div>
			<div className="flex-1 relative" style={{ minHeight: 0 }}>
				{tabs.map((tab) => (
					<div
						key={tab.id}
						className="absolute inset-0"
						style={{
							visibility: activeTabId === tab.id ? "visible" : "hidden",
							zIndex: activeTabId === tab.id ? 1 : 0,
						}}
					>
						<TerminalPanel
							ref={setTerminalRef(tab.id)}
							cwd={cwd}
							theme={theme}
							terminalStartupCommand={terminalStartupCommand}
							agentType={agentType}
							label={tab.label}
							sessionKey={
								sessionKey ? `${sessionKey}::${tab.label}` : undefined
							}
						/>
					</div>
				))}
			</div>
		</div>
	);
});
