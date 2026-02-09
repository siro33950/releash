import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("App", () => {
	it("renders manager screen by default", () => {
		render(<App />);
		expect(screen.getByText("No git repository detected")).toBeInTheDocument();
	});
});
