import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Breadcrumb } from "./Breadcrumb";

vi.mock("@react-symbols/icons/utils", () => ({
	FileIcon: ({
		fileName,
		className,
	}: {
		fileName: string;
		className?: string;
	}) => (
		<span
			data-testid="file-icon"
			data-filename={fileName}
			className={className}
		/>
	),
	FolderIcon: ({
		folderName,
		className,
	}: {
		folderName: string;
		className?: string;
	}) => (
		<span
			data-testid="folder-icon"
			data-foldername={folderName}
			className={className}
		/>
	),
}));

describe("Breadcrumb", () => {
	it("should render nothing when rootPath is null", () => {
		const { container } = render(
			<Breadcrumb rootPath={null} filePath="/root/src/file.ts" />,
		);
		expect(container.firstChild).toBeNull();
	});

	it("should render nothing when filePath is null", () => {
		const { container } = render(
			<Breadcrumb rootPath="/root" filePath={null} />,
		);
		expect(container.firstChild).toBeNull();
	});

	it("should render nothing when filePath is not under rootPath", () => {
		const { container } = render(
			<Breadcrumb rootPath="/root" filePath="/other/src/file.ts" />,
		);
		expect(container.firstChild).toBeNull();
	});

	it("should render only file name for a file at root level", () => {
		render(<Breadcrumb rootPath="/root" filePath="/root/file.ts" />);

		expect(screen.getByText("file.ts")).toBeInTheDocument();
		expect(screen.getByTestId("file-icon")).toHaveAttribute(
			"data-filename",
			"file.ts",
		);
		expect(screen.queryAllByTestId("folder-icon")).toHaveLength(0);
	});

	it("should render all segments with ChevronRight separators for nested path", () => {
		render(
			<Breadcrumb rootPath="/root" filePath="/root/src/components/App.tsx" />,
		);

		expect(screen.getByText("src")).toBeInTheDocument();
		expect(screen.getByText("components")).toBeInTheDocument();
		expect(screen.getByText("App.tsx")).toBeInTheDocument();

		const folderIcons = screen.getAllByTestId("folder-icon");
		expect(folderIcons).toHaveLength(2);
		expect(folderIcons[0]).toHaveAttribute("data-foldername", "src");
		expect(folderIcons[1]).toHaveAttribute("data-foldername", "components");

		expect(screen.getByTestId("file-icon")).toHaveAttribute(
			"data-filename",
			"App.tsx",
		);
	});

	it("should use FolderIcon for directory segments and FileIcon for the last segment", () => {
		render(<Breadcrumb rootPath="/project" filePath="/project/lib/utils.ts" />);

		const folderIcons = screen.getAllByTestId("folder-icon");
		expect(folderIcons).toHaveLength(1);
		expect(folderIcons[0]).toHaveAttribute("data-foldername", "lib");

		const fileIcon = screen.getByTestId("file-icon");
		expect(fileIcon).toHaveAttribute("data-filename", "utils.ts");
	});

	it("should handle rootPath with trailing slash", () => {
		render(<Breadcrumb rootPath="/root/" filePath="/root/src/file.ts" />);

		expect(screen.getByText("src")).toBeInTheDocument();
		expect(screen.getByText("file.ts")).toBeInTheDocument();
	});
});
