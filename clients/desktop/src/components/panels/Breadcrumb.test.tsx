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
	it("should render nothing when segments is empty", () => {
		const { container } = render(<Breadcrumb segments={[]} />);
		expect(container.firstChild).toBeNull();
	});

	it("should render only file name for a single file segment", () => {
		const segments = [{ name: "file.ts", isFile: true }];
		render(<Breadcrumb segments={segments} />);

		expect(screen.getByText("file.ts")).toBeInTheDocument();
		expect(screen.getByTestId("file-icon")).toHaveAttribute(
			"data-filename",
			"file.ts",
		);
		expect(screen.queryAllByTestId("folder-icon")).toHaveLength(0);
	});

	it("should render all segments with ChevronRight separators for nested path", () => {
		const segments = [
			{ name: "src", isFile: false },
			{ name: "components", isFile: false },
			{ name: "App.tsx", isFile: true },
		];
		render(<Breadcrumb segments={segments} />);

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
		const segments = [
			{ name: "lib", isFile: false },
			{ name: "utils.ts", isFile: true },
		];
		render(<Breadcrumb segments={segments} />);

		const folderIcons = screen.getAllByTestId("folder-icon");
		expect(folderIcons).toHaveLength(1);
		expect(folderIcons[0]).toHaveAttribute("data-foldername", "lib");

		const fileIcon = screen.getByTestId("file-icon");
		expect(fileIcon).toHaveAttribute("data-filename", "utils.ts");
	});

	it("should render children in the right area", () => {
		const segments = [{ name: "file.ts", isFile: true }];
		render(
			<Breadcrumb segments={segments}>
				<span data-testid="child-content">Extra</span>
			</Breadcrumb>,
		);

		expect(screen.getByTestId("child-content")).toBeInTheDocument();
	});
});
