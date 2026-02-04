import { MonacoDiffViewer } from "./MonacoDiffViewer";

const originalCode = `function greet(name: string) {
  console.log("Hello, " + name);
}

function add(a: number, b: number) {
  return a + b;
}

greet("World");
`;

const modifiedCode = `function greet(name: string) {
  console.log(\`Hello, \${name}!\`);
}

function add(a: number, b: number): number {
  return a + b;
}

function subtract(a: number, b: number): number {
  return a - b;
}

greet("Releash");
`;

export function EditorPanel() {
	return (
		<MonacoDiffViewer
			originalContent={originalCode}
			modifiedContent={modifiedCode}
			language="typescript"
			className="h-full w-full"
		/>
	);
}
