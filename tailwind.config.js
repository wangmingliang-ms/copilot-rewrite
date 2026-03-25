/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{js,ts,jsx,tsx}"],
  theme: {
    extend: {
      colors: {
        "copilot-blue": "#0078D4",
        "copilot-blue-hover": "#106EBE",
        "copilot-purple": "#7B61FF",
        "copilot-green": "#107C10",
        surface: {
          DEFAULT: "#FFFFFF",
          dark: "#1E1E1E",
          elevated: "#F5F5F5",
        },
      },
      boxShadow: {
        toolbar: "0 2px 12px rgba(0, 0, 0, 0.15)",
        popup: "0 8px 32px rgba(0, 0, 0, 0.2)",
      },
      animation: {
        "fade-in": "fadeIn 0.15s ease-out",
        "slide-up": "slideUp 0.2s ease-out",
        "pulse-soft": "pulseSoft 1.5s ease-in-out infinite",
      },
      keyframes: {
        fadeIn: {
          "0%": { opacity: "0" },
          "100%": { opacity: "1" },
        },
        slideUp: {
          "0%": { opacity: "0", transform: "translateY(8px)" },
          "100%": { opacity: "1", transform: "translateY(0)" },
        },
        pulseSoft: {
          "0%, 100%": { opacity: "0.6" },
          "50%": { opacity: "1" },
        },
      },
    },
  },
  plugins: [],
};
