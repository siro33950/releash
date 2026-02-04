"use client";

import { useState } from "react";
import { FileTree } from "@/components/agent-scrutiny/file-tree";
import { UnifiedEditor } from "@/components/agent-scrutiny/unified-editor";
import { Terminal } from "@/components/agent-scrutiny/terminal";
import { ConsoleOutput } from "@/components/agent-scrutiny/console-output";
import { GitPanel } from "@/components/agent-scrutiny/git-panel";
import { ReviewPanel } from "@/components/agent-scrutiny/review-panel";
import { SearchPanel } from "@/components/agent-scrutiny/search-panel";
import { StatusBar } from "@/components/agent-scrutiny/status-bar";
import {
  ActivityBar,
  type ActivityView,
} from "@/components/agent-scrutiny/activity-bar";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";

export default function AgentScrutinyApp() {
  const [activeView, setActiveView] = useState<ActivityView>("explorer");
  const [selectedFile, setSelectedFile] = useState<string | null>("Button.tsx");
  const [agentStatus, setAgentStatus] = useState<"idle" | "running" | "waiting">("idle");

  const handleSendCommand = (command: string) => {
    setAgentStatus("running");
    setTimeout(() => setAgentStatus("idle"), 3000);
  };

  const renderSidebar = () => {
    switch (activeView) {
      case "explorer":
        return <FileTree selectedFile={selectedFile} onSelectFile={setSelectedFile} />;
      case "git":
        return <GitPanel />;
      case "search":
        return <SearchPanel />;
      case "review":
        return <ReviewPanel />;
      default:
        return <FileTree selectedFile={selectedFile} onSelectFile={setSelectedFile} />;
    }
  };

  return (
    <div className="h-screen flex flex-col bg-background text-foreground overflow-hidden">
      {/* Title bar */}
      <div className="h-8 flex items-center justify-between px-4 bg-sidebar border-b border-border">
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <div className="w-3 h-3 rounded-full bg-terminal-red" />
            <div className="w-3 h-3 rounded-full bg-terminal-yellow" />
            <div className="w-3 h-3 rounded-full bg-terminal-green" />
          </div>
          <span className="text-xs font-medium text-muted-foreground">agent-scrutiny</span>
        </div>
        <div className="text-xs text-muted-foreground font-mono">~/projects/my-app</div>
        <div className="w-20" />
      </div>

      {/* Main content */}
      <div className="flex-1 flex overflow-hidden">
        {/* Activity bar */}
        <ActivityBar activeView={activeView} onViewChange={setActiveView} />

        {/* Main panels */}
        <ResizablePanelGroup direction="horizontal" className="flex-1">
          {/* Left: Sidebar */}
          <ResizablePanel defaultSize={18} minSize={12} maxSize={30}>
            <div className="h-full">{renderSidebar()}</div>
          </ResizablePanel>

          <ResizableHandle className="w-px bg-border hover:bg-primary/50 transition-colors" />

          {/* Center: Unified Editor + Console */}
          <ResizablePanel defaultSize={52}>
            <ResizablePanelGroup direction="vertical">
              {/* Unified Editor */}
              <ResizablePanel defaultSize={70} minSize={30}>
                <UnifiedEditor />
              </ResizablePanel>

              <ResizableHandle className="h-px bg-border hover:bg-primary/50 transition-colors" />

              {/* Console */}
              <ResizablePanel defaultSize={30} minSize={15}>
                <ConsoleOutput />
              </ResizablePanel>
            </ResizablePanelGroup>
          </ResizablePanel>

          <ResizableHandle className="w-px bg-border hover:bg-primary/50 transition-colors" />

          {/* Right: AI Terminal */}
          <ResizablePanel defaultSize={30} minSize={20} maxSize={45}>
            <div className="h-full flex flex-col">
              <div className="px-3 py-1.5 bg-sidebar border-b border-border">
                <span className="text-xs font-medium text-muted-foreground">AI Terminal</span>
              </div>
              <div className="flex-1 overflow-hidden">
                <Terminal onSendCommand={handleSendCommand} />
              </div>
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>
      </div>

      {/* Status bar */}
      <StatusBar agentStatus={agentStatus} />
    </div>
  );
}
