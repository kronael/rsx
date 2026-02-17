import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        bg: {
          primary: "#0b0e11",
          surface: "#1e2329",
          hover: "#2b3139",
        },
        border: {
          DEFAULT: "#2b3139",
          light: "#363c45",
        },
        text: {
          primary: "#eaecef",
          secondary: "#848e9c",
          disabled: "#5e6673",
        },
        buy: "#0ecb81",
        sell: "#f6465d",
        accent: "#fcd535",
      },
      fontFamily: {
        sans: [
          "Inter",
          "system-ui",
          "-apple-system",
          "sans-serif",
        ],
        mono: [
          "JetBrains Mono",
          "Fira Code",
          "monospace",
        ],
      },
      fontSize: {
        "2xs": "0.625rem",
      },
      keyframes: {
        "flash-buy": {
          "0%": { backgroundColor: "rgba(14,203,129,0.35)" },
          "100%": { backgroundColor: "transparent" },
        },
        "flash-sell": {
          "0%": { backgroundColor: "rgba(246,70,93,0.35)" },
          "100%": { backgroundColor: "transparent" },
        },
      },
      animation: {
        "flash-buy": "flash-buy 400ms ease-out forwards",
        "flash-sell": "flash-sell 400ms ease-out forwards",
      },
    },
  },
  plugins: [],
} satisfies Config;
