"use client";

import { X, Circle } from "lucide-react";
import { cn } from "@/lib/utils";

export interface EditorTab {
  id: string;
  filename: string;
  filepath: string;
  isDirty: boolean;
  language: string;
}

interface EditorTabsProps {
  tabs: EditorTab[];
  activeTabId: string | null;
  onTabSelect: (tabId: string) => void;
  onTabClose: (tabId: string) => void;
}

export function EditorTabs({
  tabs,
  activeTabId,
  onTabSelect,
  onTabClose,
}: EditorTabsProps) {
  if (tabs.length === 0) {
    return null;
  }

  return (
    <div className="flex items-center bg-[oklch(0.11_0.005_260)] border-b border-border overflow-x-auto">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={cn(
            "group flex items-center gap-2 px-3 py-2 border-r border-border cursor-pointer min-w-0",
            "hover:bg-muted/50 transition-colors",
            activeTabId === tab.id
              ? "bg-background border-t-2 border-t-primary"
              : "bg-[oklch(0.13_0.005_260)]"
          )}
          onClick={() => onTabSelect(tab.id)}
        >
          <FileIcon language={tab.language} />
          <span
            className={cn(
              "text-sm truncate max-w-[120px]",
              activeTabId === tab.id
                ? "text-foreground"
                : "text-muted-foreground"
            )}
          >
            {tab.filename}
          </span>
          {tab.isDirty ? (
            <Circle className="h-2 w-2 fill-primary text-primary flex-shrink-0" />
          ) : (
            <button
              type="button"
              className="opacity-0 group-hover:opacity-100 hover:bg-muted rounded p-0.5 flex-shrink-0"
              onClick={(e) => {
                e.stopPropagation();
                onTabClose(tab.id);
              }}
            >
              <X className="h-3 w-3 text-muted-foreground" />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

function FileIcon({ language }: { language: string }) {
  const colors: Record<string, string> = {
    typescript: "text-blue-400",
    javascript: "text-yellow-400",
    tsx: "text-blue-400",
    jsx: "text-yellow-400",
    css: "text-pink-400",
    json: "text-yellow-500",
    markdown: "text-blue-300",
    html: "text-orange-400",
    python: "text-green-400",
    rust: "text-orange-500",
  };

  return (
    <div
      className={cn("text-xs font-mono font-bold", colors[language] || "text-muted-foreground")}
    >
      {language === "typescript" || language === "tsx"
        ? "TS"
        : language === "javascript" || language === "jsx"
          ? "JS"
          : language.slice(0, 2).toUpperCase()}
    </div>
  );
}
