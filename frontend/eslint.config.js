/**
 * ESLint 扁平配置（ESLint 9+）
 *
 * 说明：
 * - 只保留"正确性"规则；代码风格统一交给 Prettier，
 *   避免两套工具对缩进/引号/分号给出不一致要求。
 * - 原 .eslintrc.cjs 是 ESLint 8 格式，在 ESLint 9 下 pnpm lint 从未真正执行过。
 */
import js from '@eslint/js'
import globals from 'globals'
import vue from 'eslint-plugin-vue'
import tseslint from '@typescript-eslint/eslint-plugin'
import tsParser from '@typescript-eslint/parser'

const sharedRules = {
  // 关闭基础规则，交给 TS 版本处理（避免重复/误报）
  'no-unused-vars': 'off',
  'vue/multi-word-component-names': 'off',
  '@typescript-eslint/no-unused-vars': [
    'warn',
    { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
  ],
  '@typescript-eslint/no-explicit-any': 'warn',
  eqeqeq: ['error', 'always'],
  'no-console': ['warn', { allow: ['warn', 'error', 'info'] }],
}

export default [
  { ignores: ['dist/**', 'node_modules/**', 'coverage/**'] },

  js.configs.recommended,
  ...vue.configs['flat/essential'],

  {
    files: ['**/*.ts'],
    languageOptions: {
      parser: tsParser,
      ecmaVersion: 'latest',
      sourceType: 'module',
      globals: { ...globals.browser, ...globals.node },
    },
    plugins: { '@typescript-eslint': tseslint },
    rules: sharedRules,
  },

  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tsParser,
        ecmaVersion: 'latest',
        sourceType: 'module',
      },
      globals: { ...globals.browser },
    },
    plugins: { '@typescript-eslint': tseslint },
    rules: sharedRules,
  },

  {
    files: ['**/*.config.{js,ts}', 'vite.config.ts', 'vitest.config.ts'],
    languageOptions: {
      globals: { ...globals.node },
    },
  },
]
