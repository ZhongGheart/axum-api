module.exports = {
  root: true,
  env: {
    browser: true,
    es2021: true,
    node: true,
  },
  parser: 'vue-eslint-parser',
  parserOptions: {
    parser: '@typescript-eslint/parser',
    ecmaVersion: 'latest',
    sourceType: 'module',
  },
  plugins: ['@typescript-eslint'],
  extends: [
    'eslint:recommended',
    'plugin:vue/vue3-recommended',
    'plugin:@typescript-eslint/recommended',
  ],
  rules: {
    // Vue 3 允许单字组件名
    'vue/multi-word-component-names': 'off',
    // 未使用变量警告（构建时不阻止）
    '@typescript-eslint/no-unused-vars': 'warn',
    // any 类型警告
    '@typescript-eslint/no-explicit-any': 'warn',
    // 强制使用 ===
    'eqeqeq': ['error', 'always'],
    // 不允许 console.log
    'no-console': ['warn', { allow: ['warn', 'error', 'info'] }],
    // 行尾分号
    'semi': ['error', 'never'],
    // 单引号
    'quotes': ['error', 'single', { avoidEscape: true }],
    // 缩进 2 空格
    'indent': ['error', 2, { SwitchCase: 1 }],
    // 末尾逗号
    'comma-dangle': ['error', 'always-multiline'],
  },
}
