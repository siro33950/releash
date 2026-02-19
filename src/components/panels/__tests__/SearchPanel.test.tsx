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

	it("should execute search immediately when initialQuery is provided", async () => {
		const mockResult = {
			matches: [
				{
					path: "src/app.ts",
					line_number: 3,
					line_content: "const foo = 42;",
					match_start: 6,
					match_end: 9,
				},
			],
			total_matches: 1,
			truncated: false,
		};
		mockInvoke.mockResolvedValue(mockResult);

		render(<SearchPanel rootPath="/root" initialQuery="foo" focusKey={1} />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("search_files", {
				rootPath: "/root",
				pattern: "foo",
				caseSensitive: false,
				isRegex: false,
				maxResults: 1000,
			});
		});

		const input = screen.getByTestId("search-input");
		expect(input).toHaveValue("foo");
	});

	it("should not execute search when initialQuery is empty", () => {
		render(<SearchPanel rootPath="/root" initialQuery="" focusKey={1} />);

		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("should re-search when focusKey changes with the same initialQuery", async () => {
		const mockResult = {
			matches: [],
			total_matches: 0,
			truncated: false,
		};
		mockInvoke.mockResolvedValue(mockResult);

		const { rerender } = render(
			<SearchPanel rootPath="/root" initialQuery="bar" focusKey={1} />,
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		mockInvoke.mockClear();

		rerender(<SearchPanel rootPath="/root" initialQuery="bar" focusKey={2} />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("search_files", {
				rootPath: "/root",
				pattern: "bar",
				caseSensitive: false,
				isRegex: false,
				maxResults: 1000,
			});
		});
	});

	it("should not reset query when caseSensitive is toggled after initialQuery", async () => {
		mockInvoke.mockResolvedValue({
			matches: [],
			total_matches: 0,
			truncated: false,
		});

		render(<SearchPanel rootPath="/root" initialQuery="foo" focusKey={1} />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		mockInvoke.mockClear();

		const toggle = screen.getByTestId("toggle-case");
		fireEvent.click(toggle);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("search_files", {
				rootPath: "/root",
				pattern: "foo",
				caseSensitive: true,
				isRegex: false,
				maxResults: 1000,
			});
		});

		expect(screen.getByTestId("search-input")).toHaveValue("foo");
	});

	it("should not reset query when isRegex is toggled after initialQuery", async () => {
		mockInvoke.mockResolvedValue({
			matches: [],
			total_matches: 0,
			truncated: false,
		});

		render(<SearchPanel rootPath="/root" initialQuery="foo" focusKey={1} />);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});

		mockInvoke.mockClear();

		const toggle = screen.getByTestId("toggle-regex");
		fireEvent.click(toggle);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("search_files", {
				rootPath: "/root",
				pattern: "foo",
				caseSensitive: false,
				isRegex: true,
				maxResults: 1000,
			});
		});

		expect(screen.getByTestId("search-input")).toHaveValue("foo");
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
