import { FileIcon, FolderIcon } from "@react-symbols/icons/utils";
import { ChevronRight } from "lucide-react";

interface BreadcrumbProps {
	rootPath: string | null;
	filePath: string | null;
}

export function Breadcrumb({ rootPath, filePath }: BreadcrumbProps) {
	if (rootPath == null || filePath == null) {
		return null;
	}

	const normalizedRoot = rootPath.endsWith("/") ? rootPath : `${rootPath}/`;
	if (!filePath.startsWith(normalizedRoot)) {
		return null;
	}

	const relativePath = filePath.slice(normalizedRoot.length);
	const segments = relativePath.split("/");

	return (
		<nav
			data-testid="breadcrumb"
			className="flex items-center h-[26px] px-3 text-xs text-muted-foreground bg-background border-b border-border"
		>
			{segments.map((segment, index) => {
				const isLast = index === segments.length - 1;
				return (
					<span key={segment} className="flex items-center shrink-0">
						{index > 0 && (
							<ChevronRight className="h-3 w-3 mx-0.5 text-muted-foreground/50" />
						)}
						{isLast ? (
							<FileIcon fileName={segment} className="h-4 w-4 shrink-0 mr-1" />
						) : (
							<FolderIcon
								folderName={segment}
								className="h-4 w-4 shrink-0 mr-1"
							/>
						)}
						<span>{segment}</span>
					</span>
				);
			})}
		</nav>
	);
}
