// Monaco bundled locally (works offline): editor core + the four languages CPrep runs.
import * as monaco from "monaco-editor/editor";
import "monaco-editor/languages/definitions/python/register";
import "monaco-editor/languages/definitions/javascript/register";
import "monaco-editor/languages/definitions/sql/register";
import "monaco-editor/languages/definitions/java/register";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { loader } from "@monaco-editor/react";

(self as unknown as { MonacoEnvironment: unknown }).MonacoEnvironment = {
  getWorker: () => new EditorWorker(),
};

monaco.editor.defineTheme("cprep-ink", {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "comment", foreground: "6C7872", fontStyle: "italic" },
    { token: "keyword", foreground: "F0C35A" },
    { token: "string", foreground: "9CC58A" },
    { token: "number", foreground: "E58FA0" },
    { token: "type", foreground: "5B9BD5" },
  ],
  colors: {
    "editor.background": "#121916",
    "editor.foreground": "#E9E5D9",
    "editorLineNumber.foreground": "#4B5652",
    "editorLineNumber.activeForeground": "#97A39C",
    "editor.lineHighlightBackground": "#1A2320",
    "editor.selectionBackground": "#F0C35A33",
    "editorCursor.foreground": "#F0C35A",
    "editorIndentGuide.background1": "#232D2A",
    "editorWidget.background": "#1E2926",
    "editorWidget.border": "#2E3A36",
    "scrollbarSlider.background": "#E9E5D920",
  },
});

loader.config({ monaco });

export { monaco };
