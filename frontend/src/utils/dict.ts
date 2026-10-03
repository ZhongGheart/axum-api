/**
 * 字典相关的界面文案
 *
 * 抽成纯函数是为了能直接测——挂载整个字典管理页来验一句提示太重。
 */

/** 「刷新字典缓存」端点的真实返回值 */
export interface DictCacheRefresh {
  /** 实际删除的缓存键数 */
  cleared_keys: number
  /** 重新载入缓存的字典类型数 */
  reloaded_types: number
  /** 跳过的已禁用类型数 */
  skipped_disabled_types: number
}

/**
 * 拼出「刷新缓存」按钮点击后的提示
 *
 * 此前的实现是无条件 `showSuccess('缓存刷新成功')`——而后端一个键都没删。
 * 端点现在返回真实数字，这里如实转述，并且**把 0 单独说出来**：
 * "清掉 0 个键"和"清掉了 3 个键"如果都报同一句"成功"，
 * 管理员就无法区分"确实清了"与"什么都没发生"。
 */
export function buildRefreshMessage(r: DictCacheRefresh): string {
  const parts: string[] = [
    r.cleared_keys === 0
      ? '没有需要清理的缓存键'
      : `已清空 ${r.cleared_keys} 个缓存键`,
    `回填 ${r.reloaded_types} 个类型`,
  ]
  if (r.skipped_disabled_types > 0) {
    parts.push(`跳过 ${r.skipped_disabled_types} 个已禁用类型`)
  }
  return parts.join('，')
}
