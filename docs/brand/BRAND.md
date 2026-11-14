# MiControl — identidade visual

> Documento de marca do MiControl (app de desktop Windows, Tauri + Rust, centro de controlo de um
> Xiaomi Book Pro 14). Escrito para o dono decidir; números medidos, não adjectivos.
>
> **Estado:** proposta para aprovação. `src-tauri/icons/` e `src/` **não foram tocados** nesta fase.
> Tudo o que é preciso para aplicar está em `docs/brand/` e o passo de aplicação está no §10.

---

## 0. TL;DR

- **Símbolo:** um **M** de quatro traços, monoline, com os cantos exteriores cortados a 45° e o vale
  interior a **60,00° exactos**, assente na linha de base. É o monograma do nome e o desenho de um
  *detente* — o entalhe onde um valor fica fixo.
- **Ícone da app:** o M em tinta `#191B1D` sobre uma **placa accent `#87A2EA`** em squircle (raio 22%).
  Glifo escuro sobre placa clara: 6,9:1. A alternativa vazada perdeu no teste a 16 px.
- **Wordmark:** «MiControl» construído letra a letra com a mesma gramática de barras — sem `<text>`,
  sem ficheiro de fonte, sem licença a comprar.
- **Uma só cor de marca:** o accent periwinkle existe num sítio, a placa do ícone. Símbolo e wordmark
  são sempre `currentColor`, logo funcionam em claro, escuro, mono e sobre fotografia.
- **Auditoria `svg_audit.py` (heurística):** símbolo 100/100 · ícone 100/100 · lockup vertical 100/100 ·
  wordmark e lockups horizontais 99/100 · corte pequeno 100/100.

---

## 1. Personalidade

Cinco adjectivos, na ordem em que mandam:

| Adjectivo     | O que significa no desenho                                                                 |
| ------------- | ------------------------------------------------------------------------------------------ |
| **Preciso**   | Ângulos exactos (60°, 45°, 90°), espessuras iguais, nada «a olho».                          |
| **Calmo**     | Sem gradientes, sem brilhos, sem sombras. Um accent só, e só onde há estado.                 |
| **Honesto**   | Mostra o que mede. O logo não promete velocidade nem jogos; promete controlo verificável.    |
| **Técnico**   | Parece um instrumento, não um brinquedo. A referência é a folha de desenho, não o dashboard. |
| **Contido**   | Um símbolo, uma cor, uma regra. Se uma ideia precisa de legenda, não entra.                  |

**Anti-referências (o que o MiControl não é):** não é gamer (nada de chamas, engrenagens, escudos,
ventoinhas estilizadas, néon), não é Xiaomi (não imita o logo «Mi»), não é um medidor (o produto
controla, não só observa).

## 2. O motivo: o detente

O MiControl faz uma coisa que se pode dizer numa frase: **escolhe-se um valor e a máquina mantém-no.**
80% de carga, uma curva de ventoinha, um perfil ICC, um modo de desempenho. Tudo no produto é um
*setpoint* — um ponto que se fixa e que o sistema respeita.

A forma que diz isso sem desenhar um manípulo nem um mostrador é o **entalhe**: um corte recto onde
uma peça encaixa e fica. No símbolo, esse entalhe é o **vale do M** — o único sítio onde a letra se
abre e volta a fechar, a 60° exactos, com o bico a 1 unidade da linha de base.

E porque o M? Porque o nome começa por M, porque o parque de ícones do Windows é lido a 16–32 px onde
uma letra ganha a qualquer abstracção, e porque a app já mostrava um M na sidebar — há reconhecimento
a preservar, não a deitar fora.

**Relação com o tema `hairline`.** A UI é feita de linhas de 1 px, cinzas frios (hue 250), zero
sombras, um accent que só pinta estado. A marca respeita isso e acrescenta o que a UI não pode ter: a
16 px uma linha de 1,5 não sobrevive, por isso o **símbolo é a única massa sólida da identidade** — o
resto (wordmark, ícones, réguas) é linha. O contraste é intencional: a marca é o objecto; a interface é
a grelha que o mede.

**Relação com a palavra «control».** Controlar não é ter muitos botões; é ter autoridade sobre um
valor. O símbolo não mostra um painel, mostra o ponto onde a autoridade se exerce. Daí também a
disciplina do resto: uma cor, um accent, uma espessura.

## 3. Três direcções, e porque ficou esta

Construí três, desenhei as três em SVG a preto, rasterizei e olhei para elas antes de escolher.
Ficheiros em `docs/brand/concepts/`.

### A · «Detente» — calha com batente

Uma calha horizontal com um batente pendurado, separados por uma folga hairline. A ideia era o
cursor de um fader no ponto que se fixou.

**Veredicto: rejeitada.** A render ler-se como um **T** com o pé cortado — ou um malho. A folga
hairline, que era o detalhe, desaparece a 16 px, e o que sobra é uma letra que não é a inicial e não
diz nada. Um símbolo que precisa de explicação falhou.

### B · «M de precisão» — monograma monoline

O M desenhado com uma regra só: haste a 18% da altura das maiúsculas, diagonais a 60°, cantos
exteriores cortados a 45°. **Escolhida**, com duas correcções que só apareceram a olhar:

1. a primeira versão tinha um vértice alto e cantos com «orelhas» de mitra — lia-se como uma coroa.
   Resolveu-se separando as hastes do V (sem junções de traço) e baixando o vale;
2. a face plana no fundo do vale (o «detente» literal) foi testada e **retirada**: a 7 unidades de
   largura desviava as arestas 2° e a leitura era «rombudo». O detente passou a ser conceptual (o
   ângulo exacto e o bico a 1 unidade da base), não um bocado de geometria a mais.

### C · «Colarinho» — disco com entalhe radial

Um disco sólido com um entalhe radial a 45°, como um botão com marca de posição ou um veio com
chaveta. A ideia era «o controlo que se pode rodar».

**Veredicto: rejeitada.** A render lê-se como **Pac-Man** ou a cabeça de um parafuso — a porção que
sobra do entalhe parece um bico acidental. Além disso, a 16 px o disco é uma bolha: não há letra, não
há nome, não há memória.

### Sub-decisões medidas

| Decisão                      | O que foi testado                                                             | Resultado                                                              |
| ---------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| Forma da placa               | octógono · quadrado com um canto cortado · squircle · squircle com canto      | **squircle** — as outras perdem o canto a 16 px e leem-se como defeito |
| Glifo no ícone               | vazado (fundo passa) · tinta escura                                           | **tinta escura** — o vazado muda de cor com a barra e perde contraste  |
| Produto                      | M com face plana · M com bico exacto                                          | **bico exacto** — 2° de desvio liam-se como «rombudo»                  |
| Escala do M na placa         | 0,52 · 0,56 · 0,60                                                            | **0,56** — 0,60 encostava às margens, 0,52 ficava perdido              |

## 4. Construção do símbolo

Corpo da letra em unidades (não há px no mestre — o `viewBox` é a grelha):

```
altura das maiúsculas  cap   = 72
altura-x               xh    = 50      (só no wordmark)
haste                  stem  = 13      = 18,1% de cap
largura do M           w     = 78      = 1,083 × cap
diagonal interior            = 60,00°  → vale = tan(60°) × (w/2 − stem) = 45,03
espessura horizontal da diagonal = 1,15 × stem = 14,95
bico do vale                 = 70,9    (1,1 acima da linha de base)
corte dos cantos exteriores  = 5       a 45°
folga óptica do desenho      = 8       em todos os lados
```

Três regras que se mantêm se a haste ou a largura mudarem (é o que torna o desenho parametrizável em
vez de traçado à mão):

1. **A diagonal está sempre a 60°.** O vale deriva da largura e da haste, não é escolhido.
2. **O bico cai sempre ~1 unidade acima da linha de base.** Consequência de (1) e da espessura 1,15 —
   a letra nunca sai do corpo, seja qual for o peso.
3. **Os cantos exteriores são cortados a 45°, os interiores são vivos.** É a assinatura: uma letra
   maquinada, não uma letra redonda.

**Não há `<text>`, `filter`, `mask`, raster ou gradiente** em nenhum ficheiro final. O símbolo é um
único `<path>` com 17 âncoras.

**Correcções ópticas aplicadas:** a diagonal é 15% mais espessa na horizontal do que a haste (a 60°
parece mais pesada do que é); o ícone está 0,8% acima do centro geométrico; o wordmark usa
espaçamentos diferentes por par (ver §5), não um `letter-spacing` uniforme.

## 5. Wordmark

«MiControl» é desenhado, não composto: nove letras construídas com as mesmas primitivas (rectângulos e
bandas de arco), sem `<text>` e sem ficheiro de fonte. Zero dependência de licença, zero risco de a
fonte mudar debaixo dos pés.

```
altura das maiúsculas  72      altura-x  50      haste  13
M   78    i   13    C   66    o   52    n   51    t   36    r   40    o   52    l   13
```

Decisões de desenho, e o que as resolveu:

- **C de caixa alta a 35 de raio** (1 unidade de overshoot óptico). A primeira versão tinha o C à
  altura-x e lia-se «c» — o erro mais caro do processo.
- **t com a haste a subir 7 unidades acima da barra.** Com a barra no topo da haste lia-se «T».
- **r com a haste inteira e o ombro a nascer na altura-x.** Sem isso o braço flutuava.
- **Ponto do i = espessura da haste (13).** Um ponto mais fino que a haste lê-se como falha de
  impressão; mais grosso, chama atenção a mais.
- **Espaçamento por par** (`M-i` 5, `i-C` 8, `C-o` 7, `o-n` 8, `n-t` 5, `t-r` 6, `r-o` 7, `o-l` 9):
  pares com curvas pedem mais ar do que pares de hastes.

O wordmark é **todo `currentColor`**. O accent não entra na tipografia — ver §6.

## 6. Cor

| Token                  | Hex       | oklch              | Onde                                                |
| ---------------------- | --------- | ------------------ | --------------------------------------------------- |
| **accent (marca)**     | `#87A2EA` | `72% .11 268`      | placa do ícone; é o `--accent` do tema **escuro**    |
| **tinta**              | `#191B1D` | `22% .006 250`     | M no ícone, wordmark, símbolo (`--ink` do tema claro) |
| plano claro            | `#F3F5F6` | `96.8% .003 250`   | fundo do tema claro (`--bg`)                         |
| plano escuro           | `#15181A` | `20.6% .006 250`   | `--surface` do tema escuro                           |

**Porque `#87A2EA` e não o accent do tema claro (`#6581D4`).** O ícone precisa de um campo *claro*
para o glifo ser *escuro*: medido, tinta sobre `#87A2EA` dá **6,9:1**; sobre `#6581D4` daria 4,6:1.
Também é o valor que o brief do produto nomeia (`oklch(72% .11 268)`).

**Contrastes medidos (WCAG, luminância relativa):**

| Par                              | Rácio  | Leitura                                   |
| -------------------------------- | ------ | ----------------------------------------- |
| tinta `#191B1D` sobre accent     | 6,9:1  | AA para texto — o glifo do ícone é sólido  |
| accent sobre branco              | 2,5:1  | **não usar accent como texto**             |
| accent sobre `--bg` escuro       | 7,5:1  | a placa salta na barra escura              |
| accent sobre `--surface` claro   | 2,5:1  | chega para forma, não para texto           |

Regra: **o accent é uma superfície, nunca tinta.** Se precisares de periwinkle em texto, não é este
token — é um erro de hierarquia.

## 7. Ficheiros e como se regeneram

```
docs/brand/
├─ BRAND.md                     este documento
├─ preview.html                 folha de aprovação (claro/escuro, escada de tamanhos, sidebar)
├─ logo-mark.svg                símbolo (M, currentColor)                 ← mestre
├─ logo-mark-small.svg          corte pequeno (haste 22%, currentColor)   ← 16–24 px mono
├─ logo-tile.svg                ícone (placa accent + M tinta)            ← mestre do ícone
├─ logo-wordmark.svg            «MiControl» (currentColor)
├─ logo-primary.svg             lockup horizontal (placa + wordmark)
├─ logo-primary-mono.svg        lockup a uma tinta (placa com M vazado, evenodd)
├─ logo-stacked.svg             lockup vertical
├─ concepts/                    as três direcções exploradas (A, B, C), a preto
├─ icons/                       icon set pronto a copiar para src-tauri/icons
└─ tools/
   ├─ build_brand.py            geometria → SVG (stdlib, determinístico)
   ├─ rasterize.py              SVG → PNG/ICO (Chrome headless em lote)
   └─ build_preview.py          SVG → preview.html (inline + tokens do tema)
```

```bash
python docs/brand/tools/build_brand.py     # reescreve os 6 SVG
python docs/brand/tools/rasterize.py       # reescreve icons/ (PNG + ICO + tray)
python docs/brand/tools/build_preview.py   # reescreve preview.html
```

Os três correm com o Python do sistema, sem dependências externas além do Pillow (que já existe) e de
um Chrome/Edge para rasterizar.

**Ícones gerados em `docs/brand/icons/`:** `icon.ico` (7 tamanhos: 16, 24, 32, 48, 64, 128, 256),
`icon.png` 512, `32x32.png`, `64x64.png`, `128x128.png`, `128x128@2x.png` (256), o conjunto
`Square*Logo.png` + `StoreLogo.png` do MSIX (30→310) e `tray-16/24/32/48.png`.

## 8. Rasterização — método, com as armadilhas

1. **Nada de ImageMagick nesta máquina.** `convert` no PATH é o `C:\Windows\system32\convert.exe`
   (conversão de volume NTFS) — chamá-lo «para converter o SVG» não faz nada e não dá erro óbvio.
   `rsvg-convert`, Inkscape, cairo e `cairosvg` também não existem; o `pip` do venv do Hermes não está
   instalado.
2. **Renderizador: Chrome/Edge headless.** Cada SVG é embutido num HTML do tamanho exacto, com o SVG a
   100%, e o Chrome tira um screenshot com fundo transparente. É também o motor mais próximo do
   WebView2 que desenha a app.
3. **Rasterizar de raiz por tamanho, não reduzir.** Todos os tamanhos de uma família vão numa só
   página, em posições conhecidas, e são recortados por coordenadas: uma arranque do Chrome por
   família (~3 s para o conjunto todo) em vez de um por ícone. Um 16 px desenhado a 16 px tem
   anti-aliasing próprio; reduzido de 256 fica mole.
4. **`.ico` escrito à mão.** Formato ICONDIR + entradas PNG (Vista+), com cada tamanho rasterizado
   individualmente. `Pillow.save(format="ICO")` reduziria tudo a partir de uma imagem só.
5. **O `src-tauri/icons/icon.icns` fica desactualizado** se os PNG forem substituídos. O produto só
   empacota `nsis` (Windows), por isso não é bloqueador; se um dia houver macOS, o `.icns` regenera-se
   no macOS com `iconutil`.

## 9. Uso correcto

**Área de respiro:** metade da altura das maiúsculas em todos os lados (no lockup, = 36 unidades no
mestre). Nada entra nessa margem — nem texto, nem réguas, nem o bordo de um cartão.

**Tamanhos mínimos:**

| Peça                          | Mínimo | Nota                                                       |
| ----------------------------- | ------ | ---------------------------------------------------------- |
| Placa (ícone)                 | 16 px  | testada a 16, 24, 32 — lê-se                               |
| Símbolo mono (mestre)         | 32 px  | a 24 px ainda se lê, mas perde nitidez                      |
| Corte pequeno (`-small`)      | 16 px  | é o que se usa a 16–24 px (haste 22%, sem cortes nos cantos) |
| Wordmark                      | 96 px de largura | abaixo disso a haste cai a <1,5 px               |
| Lockup horizontal             | 180 px de largura | ou passar ao `logo-stacked.svg`                  |

**Fundos aprovados:** `--bg` claro, `--surface` claro, `--bg` escuro, `--surface` escuro, placa accent
(versão mono). Sobre fotografia, usar sempre a versão mono com o M vazado.

**Nunca:**

- pôr o M solto ao lado de «MiControl» — lê-se «MMiControl» (aconteceu no primeiro render deste
  trabalho); na sidebar o M vive na linha do logo, com o texto à direita;
- usar o accent como cor de texto (2,5:1);
- rodar, inclinar, contornar, sombrear ou aplicar gradiente ao símbolo;
- esticar o wordmark (a largura é consequência da construção, não um parâmetro);
- usar o M dentro de um círculo, quadrado ou «badge» que não seja a placa definida;
- recolorir a placa com uma cor que não seja o accent (o ícone perde o contraste medido).

## 10. Aplicar na app (passo pendente de aprovação)

Nesta fase **nada disto foi feito**: `src-tauri/icons/` e `src/` estão como estavam. Quando o dono
aprovar:

```bash
# 1. ícones da app (mesmos nomes, mesmo sítio)
cp docs/brand/icons/*.png docs/brand/icons/icon.ico src-tauri/icons/

# 2. favicon que o HTML já declara e não existe
mkdir -p public/icons && cp docs/brand/logo-tile.svg public/icons/icon.svg
```

```tsx
// 3. src/components/MiControlIcon.tsx — substituir o placeholder pelo símbolo novo
//    (path do logo-mark.svg; o M antigo era um polyline de 2,5 com dois pontos e uma barra)
<path fill="currentColor" d="M5 0L13 0L39 45.033L65 0L73 0L78 5L78 67L73 72L65 72L65 25.894L39 70.927L13 25.894L13 72L5 72L0 67L0 5Z" />
```

```diff
  /* 4. src/styles/globals.css — a marca a 22 px pesa 1,6× a altura do texto */
  .sidebar-logo {
-   gap: 8px;
+   gap: 10px;
  }
```

```tsx
// 5. src/pages/MainWindow.tsx
- <MiControlIcon size={22} />
+ <MiControlIcon size={16} />
```

O ponto 3 tem de ser feito com o gerador, não à mão: `python docs/brand/tools/build_brand.py` imprime
a geometria e `docs/brand/logo-mark.svg` é o resultado — copiar o `d` de lá.

**Nota sobre o favicon:** `index.html` e `landing.html` declaram `/icons/icon.svg`, mas
`public/icons/` não existe no repo. É um 404 silencioso em dev e na landing; o passo 2 resolve-o com
o ícone novo.

## 11. O que fica por decidir

| #   | Decisão                                                        | Recomendação                                                              |
| --- | -------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 1   | Aprovar a direcção B (M de precisão)                            | sim — é a que sobrevive a 16 px e a única que diz o nome                   |
| 2   | Sidebar: marca a 16 px (recomendado) ou manter 22 px            | **16 px** — a 22 px o M fica 1,6× a altura do texto e domina a linha       |
| 3   | Tray: placa (recomendado) ou M mono                             | **placa** — o M em periwinkle desaparece na barra clara; o mono em tinta desaparece na escura |
| 4   | Substituir `MiControlIcon` e aplicar os ícones                  | quando quiseres; o passo a passo está no §10                              |
| 5   | Versão empilhada para o README/landing                          | `logo-stacked.svg` já existe; escolher onde entra                          |
| 6   | Registo de marca                                                | **não foi feito nenhum estudo de anterioridade** — recomenda-se pesquisa profissional antes de uso público |

## 12. Limites deste trabalho

- **Não houve pesquisa de anterioridade nem verificação de marcas.** O M é um desenho original, mas
  originalidade não é o mesmo que disponibilidade registral.
- **A auditoria `svg_audit.py` é heurística** (100/100 no símbolo e no ícone não é um certificado: é a
  ausência de falhas que a ferramenta sabe detectar).
- **Os ícones foram vistos a 16, 24, 32, 48, 64, 128, 256 e 512 px** em fundo claro e escuro, mas não
  foram instalados numa barra de tarefas real do Windows 11 com DPI 125/150%. Vale a pena confirmar
  visualmente depois do passo §10.
- **`icon.icns` não foi regenerado** (sem ferramentas de macOS nesta máquina) e `android/`/`ios/` do
  Tauri ficaram intocados — este produto é Windows-only.