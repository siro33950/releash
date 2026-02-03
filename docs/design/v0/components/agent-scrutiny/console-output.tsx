"use client";

import { useState } from "react";
import { cn } from "@/lib/utils";
import { X, ChevronDown } from "lucide-react";

type LogLevel = "log" | "warn" | "error" | "info";

interface LogEntry {
  level: LogLevel;
  message: string;
  timestamp: string;
  source?: string;
}

const mockLogs: LogEntry[] = [
  { level: "info", message: "Build started...", timestamp: "10:23:01", source: "webpack" },
  { level: "log", message: "Compiling TypeScript", timestamp: "10:23:02", source: "tsc" },
  { level: "warn", message: "Unused variable 'tempData' in useAuth.ts:45", timestamp: "10:23:03", source: "tsc" },
  { level: "log", message: "Bundle size: 124kb (gzip: 42kb)", timestamp: "10:23:04", source: "webpack" },
  { level: "info", message: "Build completed in 2.3s", timestamp: "10:23:05", source: "webpack" },
  { level: "log", message: "Watching for file changes...", timestamp: "10:23:05", source: "webpack" },
  { level: "info", message: "src/hooks/useAuth.ts changed", timestamp: "10:24:12", source: "watcher" },
  { level: "log", message: "Rebuilding...", timestamp: "10:24:12", source: "webpack" },
  { level: "error", message: "Type error: Property 'user' does not exist on type 'AuthState'", timestamp: "10:24:13", source: "tsc" },
];

interface ConsoleOutputProps {
  className?: string;
}

export function ConsoleOutput({ className }: ConsoleOutputProps) {
  const [logs] = useState<LogEntry[]>(mockLogs);
  const [filter, setFilter] = useState<LogLevel | "all">("all");

  const filteredLogs = filter === "all" ? logs : logs.filter((l) => l.level === filter);

  const getLevelColor = (level: LogLevel) => {
    switch (level) {
      case "error":
        return "text-terminal-red";
      case "warn":
        return "text-terminal-yellow";
      case "info":
        return "text-terminal-blue";
      default:
        return "text-terminal-foreground";
    }
  };

  const getLevelBadge = (level: LogLevel) => {
    switch (level) {
      case "error":
        return "bg-terminal-red/20 text-terminal-red";
      case "warn":
        return "bg-terminal-yellow/20 text-terminal-yellow";
      case "info":
        return "bg-terminal-blue/20 text-terminal-blue";
      default:
        return "bg-muted text-muted-foreground";
    }
  };

  return (
    <div className={cn("h-full flex flex-col bg-terminal-bg", className)}>
      {/* Console header */}
      <div className="flex items-center justify-between px-3 py-1.5 border-b border-border/50 bg-card/50">
        <div className="flex items-center gap-3">
          <span className="text-xs font-medium text-muted-foreground">Console</span>
          <div className="flex items-center gap-1">
            {(["all", "error", "warn", "info", "log"] as const).map((level) => (
              <button
                key={level}
                onClick={() => setFilter(level)}
                className={cn(
                  "px-2 py-0.5 text-xs rounded transition-colors",
                  filter === level
                    ? "bg-secondary text-secondary-foreground"
                    : "text-muted-foreground hover:text-foreground"
                )}
              >
                {level === "all" ? "All" : level.charAt(0).toUpperCase() + level.slice(1)}
              </button>
            ))}
          </div>
        </div>
        <button className="p-1 hover:bg-secondary rounded">
          <X className="h-3 w-3 text-muted-foreground" />
        </button>
      </div>

      {/* Logs */}
      <div className="flex-1 overflow-auto font-mono text-xs">
        {filteredLogs.map((log, index) => (
          <div
            key={index}
            className={cn(
              "flex items-start gap-2 px-3 py-1 hover:bg-secondary/30 border-b border-border/20",
              getLevelColor(log.level)
            )}
          >
            <span className="text-muted-foreground/60 w-16 shrink-0">{log.timestamp}</span>
            <span className={cn("px-1.5 py-0.5 rounded text-[10px] uppercase shrink-0", getLevelBadge(log.level))}>
              {log.level}
            </span>
            {log.source && (
              <span className="text-muted-foreground shrink-0">[{log.source}]</span>
            )}
            <span className="flex-1 break-all">{log.message}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
