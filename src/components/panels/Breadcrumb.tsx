import { FileIcon, FolderIcon } from "@react-symbols/icons/utils";
import { ChevronRight } from "lucide-react";

export interface BreadcrumbSegment {
	name: string;
	isFile: boolean;
}

interface BreadcrumbProps {
	segments: BreadcrumbSegment[];
	children?: React.ReactNode;
}

export function Breadcrumb({ segments, children }: BreadcrumbProps) {
	if (segments.length === 0) {
		return null;
	}

	return (
		<nav
			data-testid="breadcrumb"
			className="flex items-center h-[26px] px-3 text-xs text-muted-foreground bg-background border-b border-border"
		>
			{segments.map((segment, index) => (
				<span
					key={segments
						.slice(0, index + 1)
						.map((s) => s.name)
						.join("/")}
					className="flex items-center shrink-0"
				>
					{index > 0 && (
						<ChevronRight className="h-3 w-3 mx-0.5 text-muted-foreground/50" />
					)}
					{segment.isFile ? (
						<FileIcon
							fileName={segment.name}
							className="h-4 w-4 shrink-0 mr-1"
						/>
					) : (
						<FolderIcon
							folderName={segment.name}
							className="h-4 w-4 shrink-0 mr-1"
						/>
					)}
					<span>{segment.name}</span>
				</span>
			))}
			{children && <div className="ml-auto flex items-center">{children}</div>}
		</nav>
	);
}
