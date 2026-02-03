"use client";

import { GitBranch, Circle, Wifi, WifiOff, Zap } from "lucide-react";
import { cn } from "@/lib/utils";

interface StatusBarProps {
  branch?: string;
  isConnected?: boolean;
  agentStatus?: "idle" | "running" | "waiting";
}

export function StatusBar({
  branch = "feature/ai-improvements",
  isConnected = true,
  agentStatus = "idle",
}: StatusBarProps) {
  const getAgentStatusColor = () => {
    switch (agentStatus) {
      case "running":
        return "text-terminal-green";
      case "waiting":
        return "text-terminal-yellow";
      default:
        return "text-muted-foreground";
    }
  };

  const getAgentStatusText = () => {
    switch (agentStatus) {
      case "running":
        return "Agent Running";
      case "waiting":
        return "Awaiting Input";
      default:
        return "Agent Idle";
    }
  };

  return (
    <div className="h-6 flex items-center justify-between px-3 bg-primary text-primary-foreground text-xs">
      <div className="flex items-center gap-4">
        {/* Git branch */}
        <div className="flex items-center gap-1.5">
          <GitBranch className="h-3.5 w-3.5" />
          <span>{branch}</span>
        </div>

        {/* Agent status */}
        <div className={cn("flex items-center gap-1.5", getAgentStatusColor())}>
          <Zap className="h-3.5 w-3.5" />
          <span>{getAgentStatusText()}</span>
          {agentStatus === "running" && (
            <Circle className="h-2 w-2 fill-current animate-pulse" />
          )}
        </div>
      </div>

      <div className="flex items-center gap-4">
        {/* Connection status */}
        <div className="flex items-center gap-1.5">
          {isConnected ? (
            <>
              <Wifi className="h-3.5 w-3.5 text-terminal-green" />
              <span className="text-terminal-green">Connected</span>
            </>
          ) : (
            <>
              <WifiOff className="h-3.5 w-3.5 text-terminal-red" />
              <span className="text-terminal-red">Disconnected</span>
            </>
          )}
        </div>

        {/* File info */}
        <div className="flex items-center gap-2">
          <span>UTF-8</span>
          <span>LF</span>
          <span>TypeScript React</span>
        </div>
      </div>
    </div>
  );
}
