---
title: "Shard Rabin CDC Poster"
theme: flexoki-light
font: "Noto Sans"
color_background: "#F2F2EE"
color_foreground: "#141413"
color_dark_foreground: "#52524E"
color_accent: "#1C6B48"
color_red: "#B82B21"
---

```svg
<svg viewBox="0 0 1600 900" xmlns="http://www.w3.org/2000/svg">
  <!-- Stone archival paper background -->
  <rect x="0" y="0" width="1600" height="900" fill="var(--background)"/>

  <!-- Top and bottom Swiss rules -->
  <line x1="88" y1="68" x2="1512" y2="68" stroke="var(--foreground)" stroke-width="2.5"/>
  <line x1="88" y1="832" x2="1512" y2="832" stroke="#C8C8C0" stroke-width="1.5"/>

  <!-- Full-width header band -->
  <text x="88" y="166" font-family="Noto Sans" font-size="78" font-weight="700" fill="var(--foreground)">shard</text>
  <text x="88" y="210" font-family="Noto Sans" font-size="23" fill="var(--dark-foreground)">Content-addressed, Ed25519-signed version control for ML artifacts.</text>

  <text x="1512" y="166" text-anchor="end" font-family="JetBrainsMono Nerd Font" font-size="74" font-weight="700" fill="var(--accent)">98.5%</text>
  <text x="1512" y="210" text-anchor="end" font-family="JetBrainsMono Nerd Font" font-size="17" fill="var(--dark-foreground)">Rabin CDC dedup on 1 MiB offset shift (256 MiB weights)</text>

  <line x1="88" y1="244" x2="1512" y2="244" stroke="#C8C8C0" stroke-width="1.5"/>

  <!-- Diagram title and clean legend -->
  <text x="88" y="290" font-family="Noto Sans" font-size="20" font-weight="700" fill="var(--foreground)">Prepending 1 MiB to 256 MiB of high-entropy model weights (1.00x Zstd)</text>

  <rect x="1084" y="275" width="16" height="16" fill="var(--foreground)"/>
  <text x="1108" y="289" font-family="JetBrainsMono Nerd Font" font-size="14" fill="var(--dark-foreground)">+1 MiB</text>

  <rect x="1194" y="275" width="16" height="16" fill="var(--red)"/>
  <text x="1218" y="289" font-family="JetBrainsMono Nerd Font" font-size="14" fill="var(--dark-foreground)">re-stored</text>

  <rect x="1334" y="275" width="16" height="16" fill="var(--accent)"/>
  <text x="1358" y="289" font-family="JetBrainsMono Nerd Font" font-size="14" fill="var(--dark-foreground)">deduplicated</text>

  <!-- Row 1: Fixed 4 MiB chunking -->
  <text x="88" y="346" font-family="JetBrainsMono Nerd Font" font-size="19" font-weight="700" fill="var(--red)">--chunker fixed</text>
  <text x="1512" y="346" text-anchor="end" font-family="JetBrainsMono Nerd Font" font-size="21" font-weight="700" fill="var(--red)">257.01 MiB stored (0.0% dedup)</text>

  <!-- Prepended 1 MiB block -->
  <rect x="88" y="366" width="54" height="72" fill="var(--foreground)"/>
  <text x="115" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="15" font-weight="700" fill="#FFFFFF">+1M</text>

  <!-- 11 equal fixed chunks: numbered cleanly to show uniform grid offset shift -->
  <rect x="148" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="207" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b01</text>
  <rect x="272" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="331" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b02</text>
  <rect x="396" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="455" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b03</text>
  <rect x="520" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="579" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b04</text>
  <rect x="644" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="703" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b05</text>
  <rect x="768" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="827" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b06</text>
  <rect x="892" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="951" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b07</text>
  <rect x="1016" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="1075" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b08</text>
  <rect x="1140" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="1199" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b09</text>
  <rect x="1264" y="366" width="118" height="72" fill="var(--red)"/>
  <text x="1323" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b10</text>
  <rect x="1388" y="366" width="124" height="72" fill="var(--red)"/>
  <text x="1450" y="408" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">b11 .. b65</text>

  <text x="88" y="466" font-family="Noto Sans" font-size="16" fill="var(--dark-foreground)">Fixed 4 MiB cuts shift out of phase across the entire file. Every downstream chunk hash misses.</text>

  <!-- Row 2: Rabin CDC (clean vertical separation for re-sync marker) -->
  <text x="88" y="528" font-family="JetBrainsMono Nerd Font" font-size="19" font-weight="700" fill="var(--accent)">--chunker rabin</text>
  <text x="310" y="528" font-family="JetBrainsMono Nerd Font" font-size="14" fill="var(--dark-foreground)">boundary re-sync after chunk 1</text>
  <text x="1512" y="528" text-anchor="end" font-family="JetBrainsMono Nerd Font" font-size="21" font-weight="700" fill="var(--accent)">3.74 MiB stored (98.5% dedup)</text>

  <!-- Prepended 1 MiB + 1 boundary-absorbing chunk -->
  <rect x="88" y="556" width="54" height="72" fill="var(--foreground)"/>
  <text x="115" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="15" font-weight="700" fill="#FFFFFF">+1M</text>

  <rect x="148" y="556" width="104" height="72" fill="var(--red)"/>
  <text x="200" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">re-cut</text>

  <!-- Clean boundary re-sync hairline (never touches row label!) -->
  <line x1="258" y1="544" x2="258" y2="640" stroke="var(--foreground)" stroke-width="2" stroke-dasharray="5,4"/>

  <!-- Variable-width content-defined deduplicated chunks -->
  <rect x="264" y="556" width="136" height="72" fill="var(--accent)"/>
  <text x="332" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c02</text>
  <rect x="406" y="556" width="96" height="72" fill="var(--accent)"/>
  <text x="454" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c03</text>
  <rect x="508" y="556" width="154" height="72" fill="var(--accent)"/>
  <text x="585" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c04</text>
  <rect x="668" y="556" width="114" height="72" fill="var(--accent)"/>
  <text x="725" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c05</text>
  <rect x="788" y="556" width="144" height="72" fill="var(--accent)"/>
  <text x="860" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c06</text>
  <rect x="938" y="556" width="102" height="72" fill="var(--accent)"/>
  <text x="989" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c07</text>
  <rect x="1046" y="556" width="152" height="72" fill="var(--accent)"/>
  <text x="1122" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c08</text>
  <rect x="1204" y="556" width="128" height="72" fill="var(--accent)"/>
  <text x="1268" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c09</text>
  <rect x="1338" y="556" width="174" height="72" fill="var(--accent)"/>
  <text x="1425" y="598" text-anchor="middle" font-family="JetBrainsMono Nerd Font" font-size="14" fill="#FFFFFF">c10 .. cN</text>

  <text x="88" y="662" font-family="Noto Sans" font-size="16" fill="var(--dark-foreground)">Rolling Rabin hash snaps back to the original byte boundary within one chunk. Unchanged weights stay deduplicated.</text>

  <!-- Bottom measured ledger -->
  <line x1="88" y1="704" x2="1512" y2="704" stroke="#C8C8C0" stroke-width="1.5"/>

  <text x="88" y="746" font-family="Noto Sans" font-size="15" fill="var(--dark-foreground)">256 MiB high-entropy weights</text>
  <text x="88" y="782" font-family="JetBrainsMono Nerd Font" font-size="20" font-weight="700" fill="var(--foreground)">1.00x Zstd baseline</text>

  <line x1="546" y1="722" x2="546" y2="804" stroke="#C8C8C0" stroke-width="1.5"/>
  <text x="582" y="746" font-family="Noto Sans" font-size="15" fill="var(--dark-foreground)">256 MiB JSONL + 1 MiB prepend</text>
  <text x="582" y="782" font-family="JetBrainsMono Nerd Font" font-size="20" font-weight="700" fill="var(--accent)">0.51 MiB stored (98.0%)</text>

  <line x1="1064" y1="722" x2="1064" y2="804" stroke="#C8C8C0" stroke-width="1.5"/>
  <text x="1100" y="746" font-family="Noto Sans" font-size="15" fill="var(--dark-foreground)">shard verify (Blake3 + Ed25519)</text>
  <text x="1100" y="782" font-family="JetBrainsMono Nerd Font" font-size="20" font-weight="700" fill="var(--foreground)">982 to 1,249 MB/s</text>
</svg>
```
