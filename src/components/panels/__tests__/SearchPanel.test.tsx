import { invoke } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	screen,
	waitFor,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SearchPanel } from "../SearchPanel";

vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Panel: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
	Separator: () => <div />,
}));

const mockInvoke = vi.mocked(invoke);

describe("SearchPanel", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("should show no folder message when rootPath is null", () => {
		render(<SearchPanel rootPath={null} />);
		expect(screen.getByText("No folder opened")).toBeInTheDocument();
	});

	it("should render search input", () => {
		render(<SearchPanel rootPath="/root" />);
		expect(screen.getByTestId("search-input")).toBeInTheDocument();
	});

	it("should trigger search after debounce", async () => {
		const mockResult = {
			matches: [
				{
					path: "src/main.ts",
					line_number: 1,
					line_content: 'console.log("hello")',
					match_start: 13,
					match_end: 18,
				},
			],
			total_matches: 1,
			truncated: false,
		};
		mockInvoke.mockResolvedValue(mockResult);

		render(<SearchPanel rootPath="/root" />);

		const input = screen.getByTestId("search-input");
		fireEvent.change(input, { target: { value: "hello" } });

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("search_files", {
				rootPath: "/root",
				pattern: "hello",
				caseSensitive: false,
				isRegex: false,
				maxResults: 1000,
			});
		});
	});

	it("should display results grouped by file", async () => {
		const mockResult = {
			matches: [
				{
					path: "src/main.ts",
					line_number: 1,
					line_content: 'const hello = "world"',
					match_start: 6,
					match_end: 11,
				},
				{
					path: "src/main.ts",
					line_number: 5,
					line_content: "console.log(hello)",
					match_start: 12,
					match_end: 17,
				},
			],
			total_matches: 2,
			truncated: false,
		};
		mockInvoke.mockResolvedValue(mockResult);

		render(<SearchPanel rootPath="/root" />);

		const input = screen.getByTestId("search-input");
		fireEvent.change(input, { target: { value: "hello" } });

		await waitFor(() => {
			expect(screen.getByText("src/main.ts")).toBeInTheDocument();
			expect(screen.getByText("2 results in 1 files")).toBeInTheDocument();
		});
	});

	it("should call onSelectFileAtLine when result is clicked", async () => {
		const mockResult = {
			matches: [
				{
					path: "src/main.ts",
					line_number: 10,
					line_content: "function test() {}",
					match_start: 9,
					match_end: 13,
				},
			],
			total_matches: 1,
			truncated: false,
		};
		mockInvoke.mockResolvedValue(mockResult);

		const onSelect = vi.fn();
		render(<SearchPanel rootPath="/root" onSelectFileAtLine={onSelect} />);

		const input = screen.getByTestId("search-input");
		fireEvent.change(input, { target: { value: "test" } });

		await waitFor(() => {
			expect(screen.getByText("10")).toBeInTheDocument();
		});

		await act(async () => {
			const button = screen.getByText("10").closest("button");
			if (button) fireEvent.click(button);
		});
		expect(onSelect).toHaveBeenCalledWith("src/main.ts", 10);
	});
});
