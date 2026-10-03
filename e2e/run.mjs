// e2e 套件 runner：一次跑完（或只跑指定的几个）回归套件。
//
// 用法：
//   node e2e/run.mjs                  跑全部
//   node e2e/run.mjs role-assignment  只跑名字里含该关键字的套件
//
// 每个套件是独立进程：其中一个崩了不影响其余的结论，
// 汇总里也只看得到真实跑过的套件，不会把"没跑"混成"通过"。

import { spawnSync } from 'node:child_process'
import { readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const SUITE_DIR = join(HERE, 'suites')
const filter = process.argv[2]

/** 单个套件的硬上限。正常套件跑几十秒，这个值留了很大余量 */
const SUITE_TIMEOUT_MS = Number(process.env.E2E_SUITE_TIMEOUT_MS || 300000)

const all = readdirSync(SUITE_DIR)
  .filter((f) => f.endsWith('.mjs'))
  .sort()

if (all.length === 0) {
  console.error('e2e/suites 下没有套件')
  process.exit(1)
}

const picked = filter ? all.filter((f) => f.includes(filter)) : all

if (picked.length === 0) {
  console.error('没有匹配的套件: ' + filter)
  console.error('可选: ' + all.join(', '))
  process.exit(1)
}

console.log('待跑套件 ' + picked.length + '/' + all.length + ': ' + picked.join(', '))

const results = []
for (const file of picked) {
  console.log('\n' + '='.repeat(70))
  console.log('>>> ' + file)
  console.log('='.repeat(70))

  // 硬超时：即使某个套件挂死，也不能把整轮回归永远卡在这里。
  //
  // harness 那边已经加了退出守卫（未捕获异常时先关浏览器再退出），
  // 但守卫只覆盖"抛异常"这一种死法。若浏览器进程或 CDP 卡在某个 I/O 上，
  // 事件循环既不空转也不抛错，就只能靠外部超时兜底。
  //
  // 失败套件**永不退出**是比"失败"更坏的结果：它会让人以为测试在跑，
  // 而整轮结论一个都拿不到。
  const r = spawnSync(process.execPath, [join(SUITE_DIR, file)], {
    stdio: 'inherit',
    env: process.env,
    timeout: SUITE_TIMEOUT_MS,
    killSignal: 'SIGKILL',
  })
  // signal 导致的退出码是 null；区分"被信号杀掉"与"断言失败"
  const timedOut = r.signal === 'SIGKILL' && r.status === null
  const failed = r.status !== 0
  results.push({
    file,
    failed,
    status: timedOut ? '超时' : r.status,
    signal: timedOut ? `超过 ${SUITE_TIMEOUT_MS / 1000}s 未退出` : r.signal,
  })
}

console.log('\n' + '='.repeat(70))
console.log('e2e 总汇总')
console.log('='.repeat(70))
for (const r of results) {
  const why = r.status === 0 ? '通过'
    : r.signal ? '被终止（' + r.signal + '）'
    : '失败（' + r.status + '）'
  console.log((r.failed ? 'FAIL  ' : 'PASS  ') + r.file + '  ::  ' + why)
}

const bad = results.filter((r) => r.failed).length
console.log(`\n${results.length - bad}/${results.length} 个套件通过`)

if (bad > 0) {
  console.log('\n失败排查顺序：先看该套件第一条 FAIL（后面的失败多半是它的连带），'
    + '再确认 e2e/.artifacts/shots 下的截图。')
  process.exit(1)
}
process.exit(0)
