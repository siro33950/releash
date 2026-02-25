import { invoke } from "@tauri-apps/api/core";
import { X } from "lucide-react";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
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
	tabPrefix?: string;
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
	{
		cwd,
		theme,
		terminalStartupCommand,
		agentType,
		sessionKey,
		tabPrefix = "Terminal",
	},
	ref,
) {
	const [tabs, setTabs] = useState<Tab[]>(() => [
		{ id: nextTabId(), label: `${tabPrefix} 1`, ptyId: null },
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
		const newTab: Tab = {
			id: nextTabId(),
			label: `${tabPrefix} ${num}`,
			ptyId: null,
		};
		setTabs((prev) => {
			if (prev.length >= MAX_TABS) return prev;
			setActiveTabId(newTab.id);
			return [...prev, newTab];
		});
	}, [tabPrefix]);

	const closeTab = useCallback((tabId: string) => {
		setTabs((prev) => {
			if (prev.length <= 1) return prev;
			const tab = prev.find((t) => t.id === tabId);
			if (tab?.ptyId != null) {
				invoke("kill_pty", { ptyId: tab.ptyId }).catch((err) =>
					console.warn("kill_pty failed:", err),
				);
			}
			const next = prev.filter((t) => t.id !== tabId);
			setActiveTabId((currentActive) => {
				if (currentActive !== tabId) return currentActive;
				const idx = prev.findIndex((t) => t.id === tabId);
				const fallback = prev[idx - 1] ?? prev[idx + 1];
				return fallback?.id ?? currentActive;
			});
			return next;
		});
	}, []);

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
			<Tabs
				value={activeTabId}
				onValueChange={setActiveTabId}
				className="flex flex-col h-full gap-0"
			>
				<div className="flex items-center gap-2 shrink-0 px-2 pt-2 bg-background">
					<TabsList aria-label="ターミナルタブ">
						{tabs.map((tab) => (
							<TabsTrigger key={tab.id} value={tab.id} asChild>
								<div className="gap-2">
									<span>{tab.label}</span>
									{tabs.length > 1 && (
										<button
											type="button"
											onPointerDown={(e) => e.stopPropagation()}
											onMouseDown={(e) => e.stopPropagation()}
											onClick={(e) => {
												e.stopPropagation();
												closeTab(tab.id);
											}}
											className="p-0.5 rounded hover:bg-muted-foreground/20 transition-colors shrink-0"
											aria-label={`Close ${tab.label}`}
										>
											<X className="size-3.5" />
										</button>
									)}
								</div>
							</TabsTrigger>
						))}
					</TabsList>
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
				</div>
				<div className="flex-1 relative" style={{ minHeight: 0 }}>
					{tabs.map((tab) => (
						<TabsContent
							key={tab.id}
							value={tab.id}
							forceMount
							className="absolute inset-0 m-0 data-[state=inactive]:hidden"
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
						</TabsContent>
					))}
				</div>
			</Tabs>
		</div>
	);
});
