import { defineConfig } from "vite";
import { copyFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

// Vite config cho frontend Tauri (src/ là root).
// - dev: `pnpm run dev:web` chay Vite tren 5173 (khop tauri.conf devUrl).
// - build: `pnpm run build` -> dist/ (Tauri Windows-only se tro frontendDist ve dist/).
// - ui/aliyun-captcha.js o ngoai src/ nen copy thu cong vao dist/ui/ de
//   <script src="/ui/aliyun-captcha.js"> trong index.html chay duoc.
//   File nay chi load 1 lan duy nhat; main.js dung window.wbAliyunCaptcha.solve().
export default defineConfig({
  root: "src",
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
  },
  plugins: [
    {
      name: "copy-aliyun-captcha",
      closeBundle() {
        const src = join(process.cwd(), "ui", "aliyun-captcha.js");
        const destDir = join(process.cwd(), "dist", "ui");
        mkdirSync(destDir, { recursive: true });
        copyFileSync(src, join(destDir, "aliyun-captcha.js"));
      },
    },
  ],
});
