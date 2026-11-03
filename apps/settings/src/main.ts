import "@fontsource/instrument-serif/400.css";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

// Outside Tauri (the Vite dev server in a browser) the backend is mocked; see lib/devmock.ts.
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) await import("./lib/devmock");

const target = document.getElementById("app");
if (target) mount(App, { target });
