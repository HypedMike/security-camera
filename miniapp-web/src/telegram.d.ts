// The small part of Telegram's WebApp API the app uses.
// https://core.telegram.org/bots/webapps
interface TelegramWebApp {
  initData: string
  ready(): void
  expand(): void
}

interface Window {
  Telegram?: { WebApp?: TelegramWebApp }
}
