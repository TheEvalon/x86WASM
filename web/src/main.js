import init, { run_hello_rom, expected_hello } from "../pkg/emulator_web.js";

const out = document.getElementById("out");
const btn = document.getElementById("run");

async function boot() {
  await init();
  btn.disabled = false;
}

btn.addEventListener("click", async () => {
  btn.disabled = true;
  out.classList.remove("ok");
  out.textContent = "Running…";
  try {
    const text = run_hello_rom(100_000);
    const expected = expected_hello();
    out.textContent = `COM1: ${text}\nexpected: ${expected}`;
    if (text === expected) {
      out.classList.add("ok");
    }
  } catch (err) {
    out.textContent = String(err);
  } finally {
    btn.disabled = false;
  }
});

btn.disabled = true;
boot().catch((err) => {
  out.textContent = `Wasm init failed: ${err}`;
});
