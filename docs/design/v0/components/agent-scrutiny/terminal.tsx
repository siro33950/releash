"use client";

import React from "react"

import { useState, useRef, useEffect } from "react";
import { cn } from "@/lib/utils";

interface TerminalLine {
  type: "input" | "output" | "error" | "system";
  content: string;
}

const initialLines: TerminalLine[] = [
  { type: "system", content: "Claude Code v1.0.23" },
  { type: "system", content: "--dangerously-skip-permissions" },
  { type: "input", content: "$ claude \"Add type safety to useAuth hook\"" },
  { type: "output", content: "I'll add type safety to the useAuth hook." },
  { type: "output", content: "" },
  { type: "output", content: "Reading: src/hooks/useAuth.ts" },
  { type: "output", content: "Analyzing patterns..." },
  { type: "output", content: "" },
  { type: "output", content: "Improvements:" },
  { type: "output", content: "1. Add User type definition" },
  { type: "output", content: "2. Type useState hooks" },
  { type: "output", content: "3. Add loading/error states" },
  { type: "output", content: "" },
  { type: "output", content: "Writing: src/hooks/useAuth.ts" },
  { type: "output", content: "Writing: src/components/Button.tsx" },
  { type: "output", content: "Creating: src/components/Input.tsx" },
  { type: "output", content: "" },
  { type: "system", content: "Done. 3 files, +45 -8" },
];

interface TerminalProps {
  onSendCommand?: (command: string) => void;
  className?: string;
}

export function Terminal({ onSendCommand, className }: TerminalProps) {
  const [lines, setLines] = useState<TerminalLine[]>(initialLines);
  const [input, setInput] = useState("");
  const [isProcessing, setIsProcessing] = useState(false);
  const terminalRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (terminalRef.current) {
      terminalRef.current.scrollTop = terminalRef.current.scrollHeight;
    }
  }, [lines]);

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && input.trim() && !isProcessing) {
      const newLines = [...lines, { type: "input" as const, content: `$ ${input}` }];
      setLines(newLines);
      setInput("");
      setIsProcessing(true);

      setTimeout(() => {
        setLines((prev) => [
          ...prev,
          { type: "output", content: `Processing: "${input}"...` },
        ]);
      }, 300);

      setTimeout(() => {
        setLines((prev) => [
          ...prev,
          { type: "system", content: "Waiting for response..." },
        ]);
        setIsProcessing(false);
      }, 1500);

      onSendCommand?.(input);
    }
  };

  const getLineColor = (type: TerminalLine["type"]) => {
    switch (type) {
      case "input":
        return "text-terminal-green";
      case "error":
        return "text-terminal-red";
      case "system":
        return "text-terminal-blue";
      default:
        return "text-terminal-foreground";
    }
  };

  return (
    <div className={cn("h-full flex flex-col bg-terminal-bg font-mono text-sm", className)}>
      <div
        ref={terminalRef}
        className="flex-1 overflow-auto p-3"
      >
        {lines.map((line, index) => (
          <div
            key={index}
            className={cn(
              "leading-relaxed",
              getLineColor(line.type),
              line.type === "input" && "font-semibold mt-2"
            )}
          >
            {line.content || "\u00A0"}
          </div>
        ))}
        {isProcessing && (
          <div className="text-muted-foreground mt-1 animate-pulse">_</div>
        )}
      </div>

      <div className="flex items-center gap-2 px-3 py-2 border-t border-border/50">
        <span className="text-terminal-green">$</span>
        <input
          type="text"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="command..."
          disabled={isProcessing}
          className="flex-1 bg-transparent border-none outline-none text-terminal-foreground placeholder:text-muted-foreground/50"
        />
      </div>
    </div>
  );
}
