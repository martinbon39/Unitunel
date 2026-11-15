```
█▀▀▀▀▀▀▀▀▀▀▀▀▀█
█ █▀▀▀▀▀▀▀▀▀█ █   u n i t u n e l
█ █ █▀▀▀▀▀█ █ █
█ █ █  ▪  █   ▌   parle. pointe. dessine.
█ █ █▄▄▄▄▄█ █ █   l'IA reçoit tout.
█ █▄▄▄▄▄▄▄▄▄█ █
█▄▄▄▄▄▄▄▄▄▄▄▄▄█
```

Unitunel est une dictée vocale de bureau, comme Wispr Flow, qui garde aussi **ce que tu montres**.
Tu appuies sur `Ctrl+Espace`, l'écran s'enfonce dans un cadre noir, et tu expliques ta vision aussi
longtemps que tu veux, en naviguant normalement. Chaque clic est capturé et situé, tu peux dessiner
sur l'écran ou ouvrir un tableau blanc. Quand tu arrêtes, un **tunnel** est collé là où tu étais :
ta voix transcrite mot à mot et horodatée, entrecoupée de chaque clic (application, fenêtre, élément
exact sous le pointeur) et des captures annotées. Tu le donnes à ton agent.

L'app est écrite en Rust (Tauri 2) et tourne sur Windows et macOS. Elle n'a aucune fenêtre, seulement un overlay et une icône de barre système.

## Usage

| geste | effet |
|---|---|
| `Ctrl+Espace` (macOS : `⌥ Espace`) | démarre ; le rappuyer envoie |
| maintenir `Ctrl+Espace` | push-to-talk : relâcher envoie |
| clic n'importe où | capture de l'écran, point du clic marqué, élément pointé nommé |
| bord droit de l'écran | rail : `✎` dessiner · `▦` tableau blanc · `↵` envoyer · `×` annuler |
| `Échap` | quitte le dessin ou le tableau (hors dessin, Échap reste aux autres apps) |
| clic droit en dessin | revient au direct |

À l'envoi, le tunnel est copié dans le presse-papiers et collé dans la fenêtre active. Chaque tunnel
est aussi rangé dans `Documents/Unitunel/<date>/` : `tunnel.md`, les captures `.jpg`, le tableau
`.png` et l'audio `voix.wav`.

## Réglages

Au premier lancement, Unitunel écrit `config.json` (menu de l'icône › Réglages) :

```json
{
  "groq_api_key": "gsk_…",
  "language": "fr",
  "model": "whisper-large-v3",
  "vocabulary": ["Unitunel", "Claude Code"],
  "auto_paste": true,
  "output_dir": null
}
```

- `groq_api_key` : ta clé [Groq](https://console.groq.com/keys). La variable d'environnement `GROQ_API_KEY` sert aussi.
- `vocabulary` : noms propres à bien orthographier, transmis à Whisper.
- `model` : `whisper-large-v3` pour la précision, `whisper-large-v3-turbo` pour la vitesse.

Emplacement : `%APPDATA%\unitunel\config.json` sur Windows, `~/Library/Application Support/unitunel/config.json` sur macOS.

## Vitesse

L'audio est découpé pendant que tu parles, sur les silences, toutes les ~24 s, et chaque morceau part
aussitôt chez Groq. À l'arrêt, il ne reste à transcrire que les dernières secondes, même après un
tunnel de 20 minutes. Les captures sont prises dans des threads à part, et l'overlay est exclu des captures.

## macOS

Au premier lancement, autorise Unitunel dans Réglages › Confidentialité : **Micro**, **Enregistrement de
l'écran** et **Accessibilité** (pour les clics et le collage). L'app n'est pas signée : clic droit › Ouvrir.

## Build

```sh
npx @tauri-apps/cli@2 build          # Windows : .exe NSIS
npx @tauri-apps/cli@2 build --target universal-apple-darwin   # macOS : .dmg
```

Chaque push sur `main` est construit par GitHub Actions (onglet Actions, artefacts). Un tag `v*` publie une release.

## Licence

MIT
