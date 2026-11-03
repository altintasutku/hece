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
    case "cuda_runtime_missing":
      return { text: "Kaydedildi · yerel motor için CUDA bileşenini indir (cuda_runtime_missing)", error: true };
    case "no_nvidia_gpu":
      return { text: "Kaydedildi · yerel motor NVIDIA ekran kartı ister (no_nvidia_gpu)", error: true };
    default:
      return { text: `Kaydedildi · Hece'ye ulaşılamadı (${answer})`, error: true };
  }
}

/** The CUDA runtime row in the local-model section. */
export const cudaRuntime = {
  label: "CUDA",
  button: "İndir",
  cancel: "Vazgeç",
  missing: (mb: number) =>
    `Yerel motorun ekran kartı bileşeni; bir kez NVIDIA'dan indirilir (~${mb} MB, NVIDIA CUDA lisansıyla)`,
  downloading: (percent: number) => `İndiriliyor… %${percent}`,
  failed: (code: string) => `İndirilemedi (${code}); tekrar denersen kaldığı yerden devam eder`,
  noGpu: "NVIDIA ekran kartı bulunamadı; yerel motor çalışmaz. Bulut sağlayıcı seç",
};

export function daemonText(status: string): string {
  if (status === "idle") return "Çalışıyor, boşta";
  if (status === "recording") return "Kayıt yapıyor";
  if (status.startsWith("processing")) return "Dikte işleniyor";
  if (status === "not_running") return "Çalışmıyor";
  return `Durum alınamadı (${status})`;
}
