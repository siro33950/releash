import React from "react";
import ReactDOM from "react-dom/client";
import { PwaApp } from "./PwaApp";
import "../index.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
	<React.StrictMode>
		<PwaApp />
	</React.StrictMode>,
);
