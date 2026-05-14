import js from "@eslint/js";
import pluginVue from "eslint-plugin-vue";
import globals from "globals";
import { defineConfigWithVueTs, vueTsConfigs } from "@vue/eslint-config-typescript";

export default defineConfigWithVueTs(
  {
    ignores: [
      "dist/**",
      "dist-ssr/**",
      "node_modules/**",
      "src-tauri/gen/**",
      "src-tauri/target/**",
      "tmp/**",
    ],
  },
  {
    files: ["**/*.{js,mjs,cjs,ts,vue}"],
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
      },
    },
  },
  js.configs.recommended,
  pluginVue.configs["flat/essential"],
  vueTsConfigs.recommended,
  {
    rules: {
      "vue/multi-word-component-names": "off",
    },
  },
);
