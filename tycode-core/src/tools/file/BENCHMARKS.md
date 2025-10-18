## Benchmarks

Modify file easy benchmark results with different tools

### Overwrite file

| Setting | Success | Wall Time | Input Tokens | Output Tokens | Total Calls | Tool Calls | Successful Tool Calls |
|---------|---------|-----------|--------------|--------------|--------------|------------|------------------------|
| GROK-CODE1-ONESHOT | ✓ | 148.822125125s | 144749 | 19385 | 10 | 10 | 8 |
| GROK4-FAST-ONESHOT | ✓ | 199.446044042s | 99718 | 19123 | 10 | 10 | 8 |
| GEMINI25-FLASH-ONESHOT | ✓ | 114.909364s | 208644 | 20718 | 13 | 10 | 8 |
| GEMINI25-PRO-ONESHOT | ✓ | 216.410338375s | 131327 | 22243 | 12 | 12 | 10 |
| GLM4-6-ONESHOT | ✓ | 202.882009042s | 70130 | 5952 | 9 | 9 | 8 |
| QWEN3-CODER-ONESHOT | ✓ | 81.558105042s | 83615 | 7106 | 11 | 8 | 5 |
| GPT-OSS120B-ONESHOT | ✓ | 525.272368042s | 259490 | 25532 | 17 | 16 | 12 |
| GPT5-ONESHOT | ✓ | 297.065227667s | 23009 | 14379 | 4 | 4 | 3 |
| GPT5-CODEX-ONESHOT | ✓ | 164.580048416s | 49475 | 10082 | 7 | 6 | 5 |

### Search/Replace

| Setting | Success | Wall Time | Input Tokens | Output Tokens | Total Calls | Tool Calls | Successful Tool Calls |
|---------|---------|-----------|--------------|--------------|--------------|------------|------------------------|
| GROK-CODE1-ONESHOT | ✗ | 359.197757959s | 638508 | 41896 | 21 | 21 | 11 |
| GROK4-FAST-ONESHOT | ✗ | 376.1025035s | 471517 | 82642 | 14 | 14 | 118 |
| GEMINI25-FLASH-ONESHOT | ✗ | 80.802072833s | 214960 | 13069 | 21 | 19 | 16 |
| GEMINI25-PRO-ONESHOT | ✗ | 1690.506850292s | 136567 | 119626 | 21 | 8 | 7 |
| GLM4-6-ONESHOT | ✓ | 200.397049583s | 76359 | 5666 | 9 | 9 | 7 |
| QWEN3-CODER-ONESHOT | ✗ | 140.360002791s | 151190 | 5382 | 21 | 18 | 13 |
| GPT-OSS120B-ONESHOT | ✗ | 606.942605875s | 194951 | 59475 | 21 | 15 | 11 |
| GPT5-ONESHOT | ✓ | 734.698633959s | 63949 | 38467 | 7 | 7 | 5 |
| GPT5-CODEX-ONESHOT | ✗ | 965.72374575s | 155540 | 55281 | 21 | 20 | 3 |
| CLAUDE-HAIKU45-ONESHOT | ✓ | 128.773869375s | 162467 | 14857 | 16 | 16 | 15 |
| CLAUDE-SONNET45-ONESHOT | ✓ | 123.609207625s | 57599 | 7879 | 6 | 6 | 5 |

### Codex Patch

| Setting | Success | Wall Time | Input Tokens | Output Tokens | Total Calls | Tool Calls | Successful Tool Calls |
|---------|---------|-----------|--------------|--------------|--------------|------------|------------------------|
| GROK-CODE1-ONESHOT | ✗ | 251.773986083s | 80714 | 33917 | 4 | 4 | 1 |
| GROK4-FAST-ONESHOT | ✗ | 248.376882834s | 9269 | 18150 | 3 | 3 | 1 |
| GEMINI25-FLASH-ONESHOT | ✗ | 119.339940291s | 252904 | 21643 | 15 | 14 | 6 |
| GEMINI25-PRO-ONESHOT | ✓ | 281.297142125s | 200771 | 25306 | 15 | 13 | 11 |
| QWEN3-CODER-ONESHOT | ✗ | 101.161482916s | 142182 | 4281 | 21 | 19 | 2 |
| GLM4-6-ONESHOT | ✗ | 198.054205084s | 185122 | 4012 | 21 | 21 | 12 |
| GPT-OSS120B-ONESHOT | ✓ | 220.782801333s | 85144 | 23692 | 10 | 9 | 8 |
| GPT5-ONESHOT | ✓ | 835.055774791s | 163361 | 36940 | 12 | 11 | 9 |
| GPT5-CODEX-ONESHOT | ✗ | 200.192271792s | 12861 | 11026 | 3 | 3 | 2 |
| CLAUDE-HAIKU45-ONESHOT | ✗ | 146.604943208s | 260374 | 16223 | 21 | 21 | 16 |
| CLAUDE-SONNET45-ONESHOT | ✓ | 164.100458208s | 108658 | 8659 | 11 | 11 | 9 |

| Setting | Success | Wall Time | Input Tokens | Output Tokens | Total Calls | Tool Calls | Successful Tool Calls |
|---------|---------|-----------|--------------|--------------|--------------|------------|------------------------|
| GROK-CODE1-ONESHOT | ✗ | 100.901774667s | 8560 | 11059 | 2 | 2 | 1 |
| GROK4-FAST-ONESHOT | ✗ | 200.096810167s | 231910 | 45780 | 9 | 9 | 4 |
| GEMINI25-FLASH-ONESHOT | ✗ | 151.905379s | 361639 | 22433 | 21 | 19 | 8 |
| GEMINI25-PRO-ONESHOT | ✗ | 385.195920542s | 280678 | 43466 | 15 | 15 | 4 |
| QWEN3-CODER-ONESHOT | ✗ | 65.453941292s | 19232 | 4644 | 4 | 3 | 1 |
| GLM4-6-ONESHOT | ✗ | 89.712869583s | 95911 | 2962 | 12 | 12 | 1 |
| GPT-OSS120B-ONESHOT | ✓ | 323.895406708s | 223312 | 25820 | 18 | 12 | 9 |
| GPT5-ONESHOT | ✓ | 505.480417541s | 72723 | 26410 | 9 | 9 | 8 |
| GPT5-CODEX-ONESHOT | ✗ | 7.043106667s | 6239 | 248 | 2 | 2 | 1 |
| CLAUDE-HAIKU45-ONESHOT | ✓ | 154.633508417s | 351138 | 15542 | 18 | 18 | 13 |
| CLAUDE-SONNET45-ONESHOT | ✓ | 90.267546458s | 52277 | 4890 | 6 | 6 | 5 |
