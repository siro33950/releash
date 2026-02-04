import { Group, Panel, Separator } from "react-resizable-panels";
import { ActivityBar } from "@/components/layout/ActivityBar";
import { StatusBar } from "@/components/layout/StatusBar";
import { TitleBar } from "@/components/layout/TitleBar";
import { ConsolePanel } from "@/components/panels/ConsolePanel";
import { EditorPanel } from "@/components/panels/EditorPanel";
import { SidebarPanel } from "@/components/panels/SidebarPanel";
import { TerminalPanel } from "@/components/panels/TerminalPanel";

function App() {
	return (
		<div className="flex flex-col h-screen w-screen overflow-hidden">
			<TitleBar />
			<div className="flex flex-1 overflow-hidden">
				<ActivityBar activeItem="explorer" />
				<Group orientation="horizontal" className="flex-1">
					{/* Sidebar */}
					<Panel
						id="sidebar"
						defaultSize="15"
						minSize={10}
						maxSize="30"
						collapsible={false}
					>
						<SidebarPanel />
					</Panel>

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Editor + Console */}
					<Panel
						id="editor-console"
						defaultSize="55"
						minSize={20}
						collapsible={false}
					>
						<Group orientation="vertical">
							<Panel
								id="editor"
								defaultSize="70"
								minSize={20}
								collapsible={false}
							>
								<EditorPanel />
							</Panel>

							<Separator className="h-px bg-border hover:bg-primary/50 cursor-row-resize" />

							<Panel
								id="console"
								defaultSize="30"
								minSize={10}
								collapsible={false}
							>
								<ConsolePanel />
							</Panel>
						</Group>
					</Panel>

					<Separator className="w-px bg-border hover:bg-primary/50 cursor-col-resize" />

					{/* Terminal */}
					<Panel
						id="terminal"
						defaultSize="30"
						minSize={10}
						maxSize="60"
						collapsible={false}
					>
						<div className="h-full flex flex-col">
							<div className="px-3 py-1.5 bg-sidebar border-b border-border">
								<span className="text-xs font-medium text-muted-foreground">
									AI Terminal
								</span>
							</div>
							<div className="flex-1 overflow-hidden">
								<TerminalPanel />
							</div>
						</div>
					</Panel>
				</Group>
			</div>
			<StatusBar />
		</div>
	);
}

export default App;
