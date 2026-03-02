import { FileIcon, FolderIcon } from "@react-symbols/icons/utils";
import { ChevronRight } from "lucide-react";

interface BreadcrumbProps {
	rootPath: string | null;
	filePath: string | null;
	symbolPath?: string[];
	children?: React.ReactNode;
}

export function Breadcrumb({
	rootPath,
	filePath,
	symbolPath,
	children,
}: BreadcrumbProps) {
	if (rootPath == null || filePath == null) {
		return null;
	}

	const normalizedRoot = rootPath.endsWith("/") ? rootPath : `${rootPath}/`;
	if (!filePath.startsWith(normalizedRoot)) {
		return null;
	}

	const relativePath = filePath.slice(normalizedRoot.length);
	if (relativePath === "") {
		return null;
	}
	const segments = relativePath.split("/");

	return (
		<nav
			data-testid="breadcrumb"
			className="flex items-center h-[26px] px-3 text-xs text-muted-foreground bg-background border-b border-border"
		>
			{segments.map((segment, index) => {
				const isLast = index === segments.length - 1;
				return (
					<span
						key={segments.slice(0, index + 1).join("/")}
						className="flex items-center shrink-0"
					>
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
			{symbolPath?.map((sym) => (
				<span key={`sym-${sym}`} className="flex items-center shrink-0">
					<ChevronRight className="h-3 w-3 mx-0.5 text-muted-foreground/50" />
					<span className="text-foreground/80">{sym}</span>
				</span>
			))}
			{children && <div className="ml-auto flex items-center">{children}</div>}
		</nav>
	);
}
