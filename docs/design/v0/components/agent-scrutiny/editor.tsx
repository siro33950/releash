"use client";

import { cn } from "@/lib/utils";

interface EditorProps {
  filename: string | null;
}

const mockFileContents: Record<string, string> = {
  "Button.tsx": `"use client";

import { ButtonHTMLAttributes, forwardRef, type ForwardedRef } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const buttonVariants = cva(
  "inline-flex items-center justify-center rounded-md font-medium transition-colors",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-primary/90",
        secondary: "bg-secondary text-secondary-foreground hover:bg-secondary/80",
        outline: "border border-input bg-transparent hover:bg-accent",
        ghost: "hover:bg-accent hover:text-accent-foreground",
        destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
      },
      size: {
        sm: "h-8 px-3 text-xs",
        md: "h-10 px-4 text-sm",
        lg: "h-12 px-6 text-base",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "md",
    },
  }
);

interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  isLoading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, isLoading, children, ...props }, ref) => {
    return (
      <button
        ref={ref}
        className={cn(buttonVariants({ variant, size }), className)}
        disabled={isLoading}
        {...props}
      >
        {isLoading ? (
          <span className="animate-spin mr-2">...</span>
        ) : null}
        {children}
      </button>
    );
  }
);

Button.displayName = "Button";
`,
  "App.tsx": `import { Button } from "./components/Button";
import { Input } from "./components/Input";
import { useAuth } from "./hooks/useAuth";

function App() {
  const { user, loading, error, login, logout } = useAuth();

  if (loading) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <span className="animate-spin">Loading...</span>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background">
      <header className="border-b p-4">
        <nav className="flex items-center justify-between">
          <h1 className="text-xl font-bold">My App</h1>
          {user ? (
            <Button variant="ghost" onClick={logout}>
              Logout
            </Button>
          ) : (
            <Button onClick={() => login()}>Login</Button>
          )}
        </nav>
      </header>
      <main className="p-4">
        {error && (
          <div className="text-destructive mb-4">{error.message}</div>
        )}
        <h2>Welcome to the app!</h2>
      </main>
    </div>
  );
}

export default App;
`,
  "Input.tsx": `"use client";

import { forwardRef, type InputHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {}

export const Input = forwardRef<HTMLInputElement, InputProps>(
  ({ className, type, ...props }, ref) => {
    return (
      <input
        type={type}
        ref={ref}
        className={cn(
          "flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm",
          "placeholder:text-muted-foreground",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className
        )}
        {...props}
      />
    );
  }
);

Input.displayName = "Input";
`,
};

function getSyntaxHighlight(line: string) {
  // Simple syntax highlighting
  const highlights: { pattern: RegExp; className: string }[] = [
    { pattern: /^(import|export|from|const|let|var|function|return|if|else|switch|case|default)\b/g, className: "text-terminal-blue" },
    { pattern: /(".*?"|'.*?'|`.*?`)/g, className: "text-terminal-green" },
    { pattern: /\b(true|false|null|undefined)\b/g, className: "text-terminal-yellow" },
    { pattern: /(\/\/.*$)/g, className: "text-muted-foreground italic" },
    { pattern: /(\{|\}|\[|\]|\(|\))/g, className: "text-terminal-yellow" },
  ];

  let result = line;
  for (const { pattern, className } of highlights) {
    result = result.replace(pattern, `<span class="${className}">$&</span>`);
  }
  return result;
}

export function Editor({ filename }: EditorProps) {
  if (!filename) {
    return (
      <div className="h-full flex items-center justify-center bg-background text-muted-foreground">
        <div className="text-center">
          <p className="text-lg mb-2">No file selected</p>
          <p className="text-sm">Select a file from the explorer to view its contents</p>
        </div>
      </div>
    );
  }

  const content = mockFileContents[filename] || `// File: ${filename}\n// Content not available`;
  const lines = content.split("\n");

  return (
    <div className="h-full flex flex-col bg-background">
      {/* Tab bar */}
      <div className="flex items-center bg-card border-b border-border">
        <div className="flex items-center gap-2 px-4 py-2 bg-background border-r border-border">
          <span className="text-sm font-mono">{filename}</span>
          <span className="text-status-modified text-xs">M</span>
        </div>
      </div>

      {/* Editor content */}
      <div className="flex-1 overflow-auto">
        <table className="w-full text-sm font-mono">
          <tbody>
            {lines.map((line, index) => (
              <tr key={index} className="hover:bg-muted/30">
                <td className="w-12 text-right pr-4 text-muted-foreground select-none border-r border-border sticky left-0 bg-background">
                  {index + 1}
                </td>
                <td className="pl-4 pr-8">
                  <pre
                    className={cn("whitespace-pre text-foreground")}
                    dangerouslySetInnerHTML={{
                      __html: getSyntaxHighlight(line) || "&nbsp;",
                    }}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
