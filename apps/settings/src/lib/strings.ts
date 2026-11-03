// All UI text, one table per language. English is the default; Turkish is used when the system UI
// language is Turkish. WebView2 sets `navigator.language` from the Windows display language, so
// the window follows the same choice as the daemon's tray and toasts (DESIGN.md: Language).
// Section titles are pre-split into syllables (DESIGN.md: section title).

import type { Note } from "./save";

export type SectionId = "provider" | "shortcuts" | "vocabulary" | "about";

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
      id: "vocabulary",
      nav: "Vocabulary",
      syllables: ["Vo", "cab", "u", "lar", "y"],
      lede: "Names and terms Hece should spell your way. Higher in the list wins when space runs out.",
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
      case "cuda_runtime_missing":
        return { text: "Saved · download the CUDA runtime for the local engine (cuda_runtime_missing)", error: true };
      case "no_nvidia_gpu":
        return { text: "Saved · the local engine needs an NVIDIA graphics card (no_nvidia_gpu)", error: true };
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
    // The only network call Hece makes on its own.
    updates: "Check for new versions",
    updatesHelp: "Once a day Hece asks GitHub whether a new version exists; nothing about you is sent",
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

  // The CUDA runtime row in the local-model section (shown only when it is missing).
  cuda: {
    label: "CUDA runtime",
    missing: (mb: number) =>
      `The local engine's graphics card part, downloaded once from NVIDIA (~${mb} MB, under NVIDIA's CUDA license)`,
    noGpu: "No NVIDIA graphics card found, so the local engine cannot run here. Pick a cloud provider",
  },

  shortcuts: {
    record: "Record",
    recordHelp: "Starts a dictation in any app",
    cancel: "Cancel",
    cancelHelp: "Cancels the running recording or the latest dictation",
    change: "Change",
    reset: "Reset",
    press: "Press the new shortcut",
    pressHelp: "Esc keeps the current one",
    changeLabel: (action: string) => `Change the ${action.toLowerCase()} shortcut`,
    unsupportedKey: "Use a letter, digit, F-key, Space or a navigation key (hotkey_invalid)",
    inUse: (combo: string) => `${combo} is used by another app; kept the previous one (hotkey_in_use)`,
    hotkeyError(message: string): string {
      const code = /\((hotkey_\w+)\)$/.exec(message)?.[1];
      switch (code) {
        case "hotkey_no_modifier":
          return "Add Ctrl, Alt or Win; only F13–F24 work alone (hotkey_no_modifier)";
        case "hotkey_reserved":
          return "Windows keeps this one, or Hece pastes with it (hotkey_reserved)";
        case "hotkey_duplicate":
          return "Record and cancel need different shortcuts (hotkey_duplicate)";
        case "hotkey_invalid":
          return "Not a usable shortcut (hotkey_invalid)";
        default:
          return message;
      }
    },
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

  vocabulary: {
    add: "Add a term",
    addHelp: "A name, product or word Hece keeps getting wrong",
    placeholder: "e.g. Kubernetes",
    addButton: "Add",
    terms: "Terms",
    count: (terms: number, fit: number) =>
      terms === 0 ? "None yet" : fit >= terms ? `${terms} in use` : `${terms} terms; the first ${fit} are used (length limit)`,
    empty: "The list is empty. Add the names and jargon you say most.",
    termLabel: (n: number) => `Term ${n}`,
    moveUp: "Move up",
    moveDown: "Move down",
    remove: "Remove",
    duplicate: "Already in the list; skipped",
    invalid: "Skipped: up to 40 characters, no commas, not starting with # or !",
    overBudget: "Past the length limit; not used",
    file: "File",
    fileHelp: (path: string) => path,
    other: "Another file",
    pathPlaceholder: "C:\\path\\to\\terms.txt",
    use: "Use",
    import: "Import",
    cancel: "Cancel",
    backToDefault: "Use default",
    imported: (n: number) => (n === 1 ? "Imported 1 term" : `Imported ${n} terms`),
    failed: (code: string) =>
      code === "vocabulary_unreadable" ? "That file cannot be read (vocabulary_unreadable)" : `Could not save (${code})`,
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
      id: "vocabulary",
      nav: "Sözlük",
      syllables: ["Söz", "lük"],
      lede: "Hece'nin senin yazdığın gibi yazmasını istediğin adlar ve terimler. Yer yetmezse üstteki kazanır.",
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
      case "cuda_runtime_missing":
        return { text: "Kaydedildi · yerel motor için CUDA bileşenini indir (cuda_runtime_missing)", error: true };
      case "no_nvidia_gpu":
        return { text: "Kaydedildi · yerel motor NVIDIA ekran kartı ister (no_nvidia_gpu)", error: true };
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
    updates: "Yeni sürümleri denetle",
    updatesHelp: "Hece günde bir kez GitHub'a yeni sürüm olup olmadığını sorar; senin hakkında hiçbir şey gönderilmez",
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

  cuda: {
    label: "CUDA bileşeni",
    missing: (mb: number) =>
      `Yerel motorun ekran kartı bileşeni; bir kez NVIDIA'dan indirilir (~${mb} MB, NVIDIA CUDA lisansıyla)`,
    noGpu: "NVIDIA ekran kartı bulunamadı, yerel motor burada çalışmaz. Bulut sağlayıcı seç",
  },

  shortcuts: {
    record: "Kaydet",
    recordHelp: "Her uygulamada dikteyi başlatır",
    cancel: "Vazgeç",
    cancelHelp: "Süren kaydı ya da son dikteyi iptal eder",
    change: "Değiştir",
    reset: "Sıfırla",
    press: "Yeni kısayola bas",
    pressHelp: "Esc ile mevcut kısayol kalır",
    changeLabel: (action) => `${action} kısayolunu değiştir`,
    unsupportedKey: "Harf, rakam, F tuşu, Space ya da bir yön tuşu kullan (hotkey_invalid)",
    inUse: (combo) => `${combo} başka bir uygulamada kullanılıyor; öncekisi kaldı (hotkey_in_use)`,
    hotkeyError(message) {
      const code = /\((hotkey_\w+)\)$/.exec(message)?.[1];
      switch (code) {
        case "hotkey_no_modifier":
          return "Ctrl, Alt ya da Win ekle; tek başına yalnız F13–F24 olur (hotkey_no_modifier)";
        case "hotkey_reserved":
          return "Bunu Windows kullanıyor ya da Hece onunla yapıştırıyor (hotkey_reserved)";
        case "hotkey_duplicate":
          return "Kaydet ve vazgeç için farklı kısayollar gerekir (hotkey_duplicate)";
        case "hotkey_invalid":
          return "Kullanılabilir bir kısayol değil (hotkey_invalid)";
        default:
          return message;
      }
    },
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

  vocabulary: {
    add: "Terim ekle",
    addHelp: "Hece'nin hep yanlış yazdığı bir ad, ürün ya da sözcük",
    placeholder: "örn. Kubernetes",
    addButton: "Ekle",
    terms: "Terimler",
    count: (terms, fit) =>
      terms === 0 ? "Henüz yok" : fit >= terms ? `${terms} terim kullanımda` : `${terms} terim; uzunluk sınırı yüzünden ilk ${fit} tanesi kullanılıyor`,
    empty: "Liste boş. En çok söylediğin adları ve terimleri ekle.",
    termLabel: (n) => `Terim ${n}`,
    moveUp: "Yukarı taşı",
    moveDown: "Aşağı taşı",
    remove: "Kaldır",
    duplicate: "Listede zaten var; atlandı",
    invalid: "Atlandı: en çok 40 karakter, virgülsüz, # ya da ! ile başlamayan",
    overBudget: "Uzunluk sınırının dışında; kullanılmıyor",
    file: "Dosya",
    fileHelp: (path) => path,
    other: "Başka dosya",
    pathPlaceholder: "C:\\yol\\terimler.txt",
    use: "Kullan",
    import: "İçe aktar",
    cancel: "Vazgeç",
    backToDefault: "Varsayılana dön",
    imported: (n) => `${n} terim içe aktarıldı`,
    failed: (code) =>
      code === "vocabulary_unreadable" ? "Bu dosya okunamıyor (vocabulary_unreadable)" : `Kaydedilemedi (${code})`,
  },
};

/** Turkish only when the system UI language is Turkish; English otherwise. */
export const t: Strings = navigator.language.toLowerCase().startsWith("tr") ? tr : en;
document.documentElement.lang = t.lang;
