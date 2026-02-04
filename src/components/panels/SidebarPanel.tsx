import { ChevronRight, File, Folder } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";

interface FileTreeItemProps {
	name: string;
	isFolder?: boolean;
	level?: number;
}

function FileTreeItem({ name, isFolder, level = 0 }: FileTreeItemProps) {
	return (
		<div
			className={cn(
				"flex items-center gap-1 py-1 px-2 text-sm cursor-pointer rounded",
				"text-sidebar-foreground hover:bg-sidebar-accent",
			)}
			style={{ paddingLeft: `${level * 12 + 8}px` }}
		>
			{isFolder ? (
				<>
					<ChevronRight className="size-4 text-muted-foreground" />
					<Folder className="size-4 text-muted-foreground" />
				</>
			) : (
				<>
					<span className="w-4" />
					<File className="size-4 text-muted-foreground" />
				</>
			)}
			<span>{name}</span>
		</div>
	);
}

export function SidebarPanel() {
	return (
		<ScrollArea className="h-full bg-sidebar">
			<div className="p-2">
				<div className="text-xs uppercase font-semibold text-muted-foreground mb-2 px-2">
					Explorer
				</div>
				<FileTreeItem name="src" isFolder level={0} />
				<FileTreeItem name="App.tsx" level={1} />
				<FileTreeItem name="main.tsx" level={1} />
				<FileTreeItem name="components" isFolder level={1} />
				<FileTreeItem name="package.json" level={0} />
			</div>
		</ScrollArea>
	);
}
