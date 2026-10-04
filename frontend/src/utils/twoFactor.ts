/**
 * 两步验证的纯函数工具（v0.25.0）
 *
 * 收在这里而不是散在视图里的理由：恢复码归一化、密钥分组这类规则
 * 是**协议的一半**——用户从纸质备份里手抄回来的码能不能认，
 * 取决于前端归一化与后端 `hash_recovery_code` 是否同一套规则。
 * 有了单测，改一边就立刻知道有没有对不上。
 */
import QRCode from 'qrcode'

/**
 * 把 `n-input-otp` 的绑定值拼成字符串
 *
 * **这个坑踩过**：`n-input-otp` 的 `value` 类型是 `string[]`（每格一个字符），
 * 不是我们习惯的单个字符串。写成 `ref('')` 之后界面照常显示、按钮照常可点，
 * 直到后端回 `code: invalid type: sequence, expected a string` 才暴露——
 * 而 `vue-tsc` 在这里没报错，所以只能靠"提交前统一过一道"来兜。
 *
 * 两处用到（登录第二步、绑定确认），收在这里是因为这个类型认知本身
 * 就是这个工具模块的一部分，不该让每个调用点各写一次 `.join('')`。
 */
export function joinOtp(cells: string[] | null): string {
  // 空态是 null 而不是 []，直接 .join 会抛
  return cells ? cells.join('') : ''
}

/**
 * 归一化用户输入的恢复码
 *
 * 与后端 `hash_recovery_code` 的归一化保持一致：转大写、去掉 `-` 和空格。
 * 这里刻意把所有空白（制表符、换行）一并去掉——**只多认输入、
 * 不改语义**：归一化后的干净串才是提交给后端的东西，
 * 摘要仍是同一套规则算出来的。
 *
 * **去掉空格和连字符是有实际收益的**：恢复码会被手抄到纸上，
 * 用户分两行写、中间带个空格是常态。归一化只发生在**入口**，
 * 存库的仍是原样摘要。
 */
export function normalizeRecoveryCode(raw: string): string {
  return raw.trim().toUpperCase().replace(/[\s-]/g, '')
}

/**
 * 判断恢复码是否"看起来填全了"
 *
 * 只用于拦掉明显不完整的输入（少填一半就点提交），**不是校验**：
 * 对错只有服务端能判。刻意不写死长度——前端把长度写死、
 * 后端改了位数，两边就会有一边永远不让提交。
 */
export function isRecoveryCodeFilled(raw: string): boolean {
  return normalizeRecoveryCode(raw).length >= 8
}

/**
 * 把 Base32 密钥按 4 位分组显示
 *
 * 验证器 App 手动输密钥时，连续 32 位字符极容易看错行。
 * 显示用空格分组，**提交的仍是去空格的原串**——
 * 这里只做展示，不参与请求体。
 */
export function groupSecret(secret: string, size = 4): string {
  const clean = secret.replace(/\s/g, '')
  const chunks: string[] = []
  for (let i = 0; i < clean.length; i += size) {
    chunks.push(clean.slice(i, i + size))
  }
  return chunks.join(' ')
}

/**
 * 把 `otpauth://` URI 渲染成二维码 Data URL
 *
 * 前端渲染而不是让后端出图：后端的 `totp-rs` 生成二维码要走
 * `image` crate（整个二进制依赖树只为画一张图），而且返回二进制
 * 会让"扫码失败"这种问题多一层排查。Base64 的 Data URL 直接
 * 塞进 `<img src>`，没有额外请求也没有额外依赖。
 */
export async function renderQrDataUrl(uri: string, size = 200): Promise<string> {
  return QRCode.toDataURL(uri, {
    width: size,
    margin: 2,
    errorCorrectionLevel: 'M',
    color: { dark: '#1f2937', light: '#ffffff' },
  })
}

/**
 * 把恢复码批次拼成可复制的纯文本
 *
 * 只给"复制全部"用。用户往往要把这批码存进密码管理器，
 * 一个个手抄既慢又容易错抄。
 */
export function formatRecoveryCodes(codes: string[]): string {
  return codes.join('\n')
}
