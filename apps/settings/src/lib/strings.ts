// All UI text, one table per language. English is the default; Turkish is used when the system UI
// language is Turkish. WebView2 sets `navigator.language` from the Windows display language, so
// the window follows the same choice as the daemon's tray and toasts (DESIGN.md: Language).
// Section titles are pre-split into syllables (DESIGN.md: section title).

import type { Note } from "./save";

export type SectionId = "provider" | "shortcuts" | "about";

export interface Section {
  id: SectionId;
  nav: string;
  syllables: string[];
  lede: string;
}

const en = {
  lang: "en",
  sections: [
    {
      id: "provider",
      nav: "Provider",
      syllables: ["Pro", "vid", "er"],
      lede: "The engine that turns your speech into text. Local first, cloud if you want.",
    },
    {
      id: "shortcuts",
      nav: "Shortcuts",
      syllables: ["Short", "cuts"],
      lede: "How you start and stop a recording, and which microphone listens.",
    },
    {
      id: "about",
      nav: "About",
      syllables: ["A", "bout"],
      lede: "The settings file and what Hece is doing right now.",
    },
  ] as Section[],
  navLabel: "Sections",
  navSelectLabel: "Section",
  notLoaded: "Settings are not loaded yet",
  notRunning: "Hece is not running. Start it once the settings look right.",
  start: "Start",
  starting: "Starting…",
  readFailed: (e: string) => `Could not read the settings: ${e}`,

  /** What the row shows after a save, by the daemon's reload answer. */
  savedNote(answer: string): Note {
    switch (answer) {
      case "ok":
        return { text: "Saved", error: false };
      case "not_running":
        return { text: "Saved · applies when Hece starts", error: false };
      case "busy":
        return { text: "Saved · restart Hece when the dictation finishes", error: false };
      case "invalid_config":
        return { text: "Saved, but Hece cannot start with these settings (invalid_config)", error: true };
      default:
        return { text: `Saved · could not reach Hece (${answer})`, error: true };
    }
  },
  daemonText(status: string): string {
    if (status === "idle") return "Running, idle";
    if (status === "recording") return "Recording";
    if (status.startsWith("processing")) return "Transcribing a dictation";
    if (status === "not_running") return "Not running";
    return `Status unavailable (${status})`;
  },

  about: {
    status: "Status",
    configFile: "Settings file",
    commentsKept: "Comments are kept",
    quote: "Every setting saves at once; a running Hece reloads it.",
  },

  key: {
    label: "API key",
    help: "Stored in Windows Credential Manager, never shown here",
    placeholder: "Paste the key",
    save: "Save",
    cancel: "Cancel",
    stored: "Stored ✓",
    change: "Change",
    remove: "Delete",
    add: "Add key",
  },

  provider: {
    engine: "Engine",
    engineHelp: "Local: on your graphics card, audio never leaves the computer. Cloud: needs a key, audio goes to the provider.",
    local: "Local (GPU)",
    groq: "Groq (cloud)",
    openai: "OpenAI (cloud)",
    deepgram: "Deepgram (cloud)",
    custom: "OpenAI-compatible server",
    keyHelp: {
      groq: "Get it at console.groq.com/keys; stored in Credential Manager",
      openai: "Get it at platform.openai.com/api-keys; stored in Credential Manager",
      deepgram: "Get it at console.deepgram.com; stored in Credential Manager",
    } as Record<string, string>,
    groqModelHelp: "large-v3 is more accurate, turbo is faster and cheaper",
    model: "Model",
    downloading: (p: number) => `Downloading… ${p}%`,
    downloadFailed: (code: string) => `Download failed (${code}); trying again resumes where it stopped`,
    downloadOnce: (gb: string) => `Downloaded once (${gb} GB), then works offline`,
    localHelp: "Transcribes on this computer, on the graphics card",
    customFolder: "Your own folder",
    cancel: "Cancel",
    download: "Download",
    url: "Address",
    urlHelp: "The server's …/v1 address (speaches, whisper.cpp, LocalAI)",
    modelNameHelp: "The provider's model name, for example whisper-1",
    name: "Name",
    nameHelp: "Shown in messages and used for the stored key",
    language: "Language",
    languageHelp: "The language you speak; pick it if auto-detect guesses wrong",
    languages: [
      ["", "Auto-detect"],
      ["en", "English"],
      ["tr", "Turkish"],
    ] as [string, string][],
    fallback: "Cloud fallback",
    fallbackHelp: "Let a cloud provider take over when the local engine fails",
  },

  shortcuts: {
    record: "Record",
    recordHelp: "A shortcut Windows does not use; cannot be changed yet",
    cancel: "Cancel",
    cancelHelp: "Cancels the running recording or the latest dictation",
    mode: "Recording mode",
    modeHelp: "Hold: ends when you let go. Toggle: ends on the second press.",
    hold: "Hold",
    toggle: "Toggle",
    microphone: "Microphone",
    microphoneHelp: "Default is the input device Windows picked",
    windowsDefault: "Windows default",
    overlay: "Recording indicator",
    overlayHelp: "An orb in the top-right corner that reacts to your voice while recording",
    sounds: "Sound cues",
    soundsHelp: "A short, soft sound when recording starts, stops or is cancelled",
    maxLength: "Longest recording",
    maxLengthHelp: "Recording stops by itself after this long (at most 11 min)",
    minutes: (m: number) => (m === 1 ? "1 minute" : `${m} minutes`),
  },
};

export type Strings = typeof en;

const tr: Strings = {
  lang: "tr",
  sections: [
    {
      id: "provider",
      nav: "Sağlayıcı",
      syllables: ["Sağ", "la", "yı", "cı"],
      lede: "Konuşmanı metne çeviren motor. Önce yerel, istersen bulut.",
    },
    {
      id: "shortcuts",
      nav: "Kısayollar",
      syllables: ["Kı", "sa", "yol", "lar"],
      lede: "Kaydı nasıl başlatıp bitirdiğin ve hangi mikrofonun dinlendiği.",
    },
    {
      id: "about",
      nav: "Hakkında",
      syllables: ["Hak", "kın", "da"],
      lede: "Ayar dosyası ve Hece'nin şu anki durumu.",
    },
  ],
  navLabel: "Bölümler",
  navSelectLabel: "Bölüm",
  notLoaded: "Ayarlar henüz okunmadı",
  notRunning: "Hece çalışmıyor. Ayarları düzelttikten sonra başlat.",
  start: "Başlat",
  starting: "Başlatılıyor…",
  readFailed: (e) => `Ayarlar okunamadı: ${e}`,

  savedNote(answer) {
    switch (answer) {
      case "ok":
        return { text: "Kaydedildi", error: false };
      case "not_running":
        return { text: "Kaydedildi · Hece açılınca geçerli olur", error: false };
      case "busy":
        return { text: "Kaydedildi · dikte bitince Hece'yi yeniden başlat", error: false };
      case "invalid_config":
        return { text: "Kaydedildi ama Hece bu ayarlarla başlayamıyor (invalid_config)", error: true };
      default:
        return { text: `Kaydedildi · Hece'ye ulaşılamadı (${answer})`, error: true };
    }
  },
  daemonText(status) {
    if (status === "idle") return "Çalışıyor, boşta";
    if (status === "recording") return "Kayıt yapıyor";
    if (status.startsWith("processing")) return "Dikte işleniyor";
    if (status === "not_running") return "Çalışmıyor";
    return `Durum alınamadı (${status})`;
  },

  about: {
    status: "Durum",
    configFile: "Ayar dosyası",
    commentsKept: "Yorumlar korunur",
    quote: "Her ayar hemen kaydedilir; Hece çalışıyorsa yeniden yüklenir.",
  },

  key: {
    label: "API anahtarı",
    help: "Windows Kimlik Bilgisi Yöneticisi'nde saklanır, burada gösterilmez",
    placeholder: "Anahtarı yapıştır",
    save: "Kaydet",
    cancel: "Vazgeç",
    stored: "Kayıtlı ✓",
    change: "Değiştir",
    remove: "Sil",
    add: "Anahtar ekle",
  },

  provider: {
    engine: "Motor",
    engineHelp: "Yerel: ekran kartında, ses bilgisayardan çıkmaz. Bulut: anahtar gerekir, ses sağlayıcıya gider.",
    local: "Yerel (GPU)",
    groq: "Groq (bulut)",
    openai: "OpenAI (bulut)",
    deepgram: "Deepgram (bulut)",
    custom: "OpenAI uyumlu sunucu",
    keyHelp: {
      groq: "console.groq.com/keys adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
      openai: "platform.openai.com/api-keys adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
      deepgram: "console.deepgram.com adresinden al; Kimlik Bilgisi Yöneticisi'nde saklanır",
    },
    groqModelHelp: "large-v3 daha doğru, turbo daha hızlı ve ucuz",
    model: "Model",
    downloading: (p) => `İndiriliyor… %${p}`,
    downloadFailed: (code) => `İndirilemedi (${code}); tekrar denersen kaldığı yerden devam eder`,
    downloadOnce: (gb) => `Bir kez indirilir (${gb} GB), sonra internetsiz çalışır`,
    localHelp: "Konuşmayı bu bilgisayarda, ekran kartında yazıya çevirir",
    customFolder: "Kendi klasörün",
    cancel: "Vazgeç",
    download: "İndir",
    url: "Adres",
    urlHelp: "Sunucunun …/v1 adresi (speaches, whisper.cpp, LocalAI)",
    modelNameHelp: "Sağlayıcının model adı, örneğin whisper-1",
    name: "Ad",
    nameHelp: "Mesajlarda ve anahtar kaydında görünür",
    language: "Dil",
    languageHelp: "Konuştuğun dil; otomatik algılama yanılırsa seç",
    languages: [
      ["", "Otomatik algıla"],
      ["en", "İngilizce"],
      ["tr", "Türkçe"],
    ],
    fallback: "Buluta yedekle",
    fallbackHelp: "Yerel motor çalışmazsa bulut yedeğine geçmeye izin ver",
  },

  shortcuts: {
    record: "Kaydet",
    recordHelp: "Windows'un kullanmadığı bir kısayol; şimdilik değiştirilemez",
    cancel: "Vazgeç",
    cancelHelp: "Süren kaydı ya da son dikteyi iptal eder",
    mode: "Kayıt biçimi",
    modeHelp: "Basılı tut: bırakınca biter. Bas-bas: ikinci basışta biter.",
    hold: "Basılı tut",
    toggle: "Bas-bas",
    microphone: "Mikrofon",
    microphoneHelp: "Varsayılan, Windows'un seçtiği giriş aygıtıdır",
    windowsDefault: "Windows varsayılanı",
    overlay: "Kayıt göstergesi",
    overlayHelp: "Kayıt sırasında ekranın sağ üstünde sese tepki veren küre",
    sounds: "Sesli uyarı",
    soundsHelp: "Kayıt başlarken, biterken ve iptal edilince kısa, yumuşak bir ses",
    maxLength: "En uzun kayıt",
    maxLengthHelp: "Bu süreden sonra kayıt kendiliğinden biter (en çok 11 dk)",
    minutes: (m) => `${m} dakika`,
  },
};

/** Turkish only when the system UI language is Turkish; English otherwise. */
export const t: Strings = navigator.language.toLowerCase().startsWith("tr") ? tr : en;
document.documentElement.lang = t.lang;
