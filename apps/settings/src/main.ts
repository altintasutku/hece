import "@fontsource/instrument-serif/400.css";
import "@fontsource/instrument-serif/400-italic.css";
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");
if (target) mount(App, { target });
