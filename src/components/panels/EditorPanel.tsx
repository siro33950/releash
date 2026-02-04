import { Code } from "lucide-react";
import { cn } from "@/lib/utils";

export function EditorPanel() {
	return (
		<div
			className={cn(
				"h-full flex items-center justify-center",
				"bg-background text-muted-foreground",
			)}
		>
			<div className="text-center">
				<Code className="size-16 mx-auto mb-4 opacity-30" />
				<div className="text-lg font-medium">Editor</div>
				<div className="text-sm">Monaco Editor will be placed here</div>
			</div>
		</div>
	);
}
