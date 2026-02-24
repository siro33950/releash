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
import { TabBarContainer, TabBarItem } from "@/components/ui/tab-bar";
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
		tabCounter.current += 1;
		const num = tabCounter.current;
		setTabs((prev) => {
			if (prev.length >= MAX_TABS) return prev;
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
			const tab = tabs.find((t) => t.id === tabId);
			if (tab?.ptyId != null) {
				invoke("kill_pty", { ptyId: tab.ptyId }).catch((err) =>
					console.warn("kill_pty failed:", err),
				);
			}
			setTabs((prev) => {
				if (prev.length <= 1) return prev;
				const next = prev.filter((t) => t.id !== tabId);
				setActiveTabId((currentActive) =>
					currentActive === tabId ? next[0].id : currentActive,
				);
				return next;
			});
		},
		[tabs],
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
			<TabBarContainer ariaLabel="ターミナルタブ">
				{tabs.map((tab, index) => (
					<TabBarItem
						key={tab.id}
						id={`terminal-tab-${tab.id}`}
						isActive={activeTabId === tab.id}
						onClick={() => setActiveTabId(tab.id)}
						onClose={tabs.length > 1 ? () => closeTab(tab.id) : undefined}
						closeLabel={`Close ${tab.label}`}
						ariaControls={`terminal-panel-${tab.id}`}
						onKeyDown={(e) => {
							if (e.key === "ArrowRight") {
								e.preventDefault();
								const next = tabs[index + 1];
								if (next) {
									setActiveTabId(next.id);
									(e.currentTarget.nextElementSibling as HTMLElement)?.focus();
								}
							}
							if (e.key === "ArrowLeft") {
								e.preventDefault();
								const prev = tabs[index - 1];
								if (prev) {
									setActiveTabId(prev.id);
									(
										e.currentTarget.previousElementSibling as HTMLElement
									)?.focus();
								}
							}
						}}
					>
						<span>{tab.label}</span>
					</TabBarItem>
				))}
				{tabs.length < MAX_TABS && (
					<button
						type="button"
						onClick={addTab}
						aria-label="Add terminal tab"
						className="px-2 h-full text-sm text-muted-foreground hover:text-foreground transition-colors shrink-0"
					>
						+
					</button>
				)}
			</TabBarContainer>
			<div className="flex-1 relative" style={{ minHeight: 0 }}>
				{tabs.map((tab) => (
					<div
						key={tab.id}
						id={`terminal-panel-${tab.id}`}
						role="tabpanel"
						aria-labelledby={`terminal-tab-${tab.id}`}
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
