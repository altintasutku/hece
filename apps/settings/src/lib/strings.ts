// All UI text. Section titles are pre-split into syllables (DESIGN.md: section title).

export interface Section {
  id: "provider" | "shortcuts" | "about";
  nav: string;
  syllables: string[];
  lede: string;
}

export const sections: Section[] = [
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
];

/** The update check row (the only network call Hece makes on its own). */
export const updates = {
  label: "Yeni sürüm denetimi",
  help: "Günde bir kez GitHub'a yeni sürüm olup olmadığını sorar; senin hakkında hiçbir şey göndermez",
};

/** What the row shows after a save, by the daemon's reload answer. */
export function savedNote(answer: string): { text: string; error: boolean } {
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
}

export function daemonText(status: string): string {
  if (status === "idle") return "Çalışıyor, boşta";
  if (status === "recording") return "Kayıt yapıyor";
  if (status.startsWith("processing")) return "Dikte işleniyor";
  if (status === "not_running") return "Çalışmıyor";
  return `Durum alınamadı (${status})`;
}
