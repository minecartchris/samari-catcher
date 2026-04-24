import { h, render } from "preact";
import { Main } from "./main";
import type * as monaco from "monaco-editor";

{
  requirejs.config({ paths: { vs: "__monaco__/min/vs" } });

  (window as any).MonacoEnvironment = { // eslint-disable-line @typescript-eslint/no-explicit-any
    getWorkerUrl: (_workerId: string, _label: string) =>
      `data:text/javascript;charset=utf-8,${encodeURIComponent(`
      self.MonacoEnvironment = {
        baseUrl: "__monaco__/min/"
      };
      importScripts("__monaco__/min/vs/base/worker/workerMain.js");
    `)}`,
  } as monaco.Environment;

  const page = document.getElementById("page") as HTMLElement;
  render(<Main />, page, page.lastElementChild || undefined);
}
