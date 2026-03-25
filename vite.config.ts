import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vite options tailored for Tauri development
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // tell vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
