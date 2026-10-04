<template>
  <div class="auth-page">
    <!-- 品牌区：宽屏时占左半屏，说明这是什么系统；窄屏隐藏让表单居中 -->
    <aside class="auth-brand">
      <div class="auth-brand-inner">
        <div class="auth-logo">
          <span class="auth-logo-mark">
            <n-icon size="20"><FlashIcon /></n-icon>
          </span>
          <span class="auth-logo-text">Axum Admin</span>
        </div>
        <h2 class="auth-pitch">让账号与权限都有人能自己管</h2>
        <ul class="auth-points">
          <li v-for="point in highlights" :key="point.title">
            <span class="auth-point-dot" />
            <span>
              <strong>{{ point.title }}</strong>
              <span class="auth-point-desc">{{ point.desc }}</span>
            </span>
          </li>
        </ul>
      </div>
    </aside>

    <!-- 表单区 -->
    <main class="auth-form-area">
      <div class="auth-card">
        <slot />
      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
/**
 * 认证页外壳（登录 / 注册共用）
 *
 * 此前两个页面各自复制了一份 `#667eea → #764ba2` 的紫渐变背景 + 一张白卡，
 * 于是同一个系统有两个入口长得不一样，而且这个紫蓝主色和后台界面里
 * 实际在用的主色并不一致——用户从登录进到系统，会有"换了个网站"的错觉。
 *
 * 现在背景与卡片只此一处，两个页面只提供表单。
 */
import { FlashOutline as FlashIcon } from '@vicons/ionicons5'

/** 品牌区的三条说明：写产品实际有的能力，不写空泛的卖点 */
const highlights = [
  { title: '自助资料与改密', desc: '用户不用找管理员就能改自己的资料' },
  { title: '角色与菜单授权', desc: '菜单、按钮、接口三层权限一致' },
  { title: '操作全程留痕', desc: '写操作进审计日志，可查可导出' },
]
</script>

<style scoped>
/*
 * 背景用中性灰 + 一层极淡的主色光晕，不用彩色渐变。
 * 渐变（尤其蓝紫渐变）会把注意力从表单上抢走，也让登录页和后台内部脱节。
 */
.auth-page {
  display: flex;
  min-height: 100vh;
  background: var(--bg-color);
}

.auth-brand {
  display: none;
  flex: 1;
  align-items: center;
  padding: 48px;
  background:
    radial-gradient(1200px 600px at 20% 10%, var(--primary-color-soft), transparent 70%),
    var(--bg-subtle);
  border-right: 1px solid var(--border-color);
}

.auth-brand-inner {
  max-width: 380px;
}

.auth-logo {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 32px;
}

.auth-logo-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-md);
  background: var(--primary-color);
  color: #fff;
}

.auth-logo-text {
  font-size: 17px;
  font-weight: 600;
  color: var(--text-primary);
}

.auth-pitch {
  font-size: 26px;
  font-weight: 600;
  line-height: 1.4;
  color: var(--text-primary);
  margin-bottom: 28px;
}

.auth-points {
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 18px;
}

.auth-points li {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}

.auth-point-dot {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  margin-top: 7px;
  border-radius: 50%;
  background: var(--primary-color);
}

.auth-points strong {
  display: block;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}

.auth-point-desc {
  font-size: 13px;
  color: var(--text-secondary);
}

.auth-form-area {
  display: flex;
  flex: 1;
  align-items: center;
  justify-content: center;
  padding: 32px 20px;
}

.auth-card {
  width: 100%;
  max-width: 400px;
  padding: 32px;
  background: var(--bg-card);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-raised);
}

@media (min-width: 960px) {
  .auth-brand {
    display: flex;
  }
}
</style>
