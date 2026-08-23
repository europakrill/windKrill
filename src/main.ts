// windKrill frontend bootstrap. M0: placeholder until the canvas
// terminal renderer lands (M1).
const app = document.querySelector<HTMLDivElement>("#app")!;

app.innerHTML = `
  <h1>windKrill</h1>
  <p>Fully open-source terminal — engine (Rust) + shell (Tauri 2 / TS).</p>
  <p>M0: repository skeleton initialized.</p>
`;
