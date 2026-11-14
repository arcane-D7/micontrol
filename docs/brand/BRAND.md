# MiControl — identidade visual

> Documento de marca do MiControl (app de desktop Windows, Tauri + Rust, centro de controlo de um
> Xiaomi Book Pro 14). Escrito para o dono decidir; números medidos, não adjectivos.
>
> **Versão 1.1 — grayscale.** O accent periwinkle `#87A2EA` foi retirado: a identidade é só cinzas
> neutros. O `preview.html` é self-contained desde esta versão (SVG inline + PNG em data: URI), para
> os ícones aparecerem em qualquer contexto, incluindo widget embutido.
>
> **Estado:** proposta para aprovação. `src-tauri/icons/` e `src/` **não foram tocados** nesta fase.
> Tudo o que é preciso para aplicar está em `docs/brand/` e o passo de aplicação está no §10.

---

## 0. TL;DR

- **Símbolo:** um **M** de quatro traços, monoline, com os cantos exteriores cortados a 45° e o vale
  interior a **60,00° exactos**, assente na linha de base. É o monograma do nome e o desenho de um
  *detente* — o entalhe onde um valor fica fixo.
- **Ícone da app:** o M em tinta `#191B1D` sobre uma **placa cinza clara `#E8E9EA`** em squircle
  (raio 22%). Contraste do glifo: **14,21:1** (AA e AAA). No tema escuro, placa `#232527` com
  M `#F2F3F4`: **13,84:1**. A alternativa vazada perdeu no teste a 16 px.
- **Wordmark:** «MiControl» construído letra a letra com a mesma gramática de barras — sem `<text>`,
  sem ficheiro de fonte, sem licença a comprar.
- **Zero cor de marca:** só cinzas neutros (croma ≤ 4/255). Não há accent, nem gradientes, nem
  sombras. Símbolo e wordmark são `currentColor`; a placa existe em duas versões (clara/escura).
- **Auditoria `svg_audit.py` (heurística):** todos os 10 SVG de marca e o `preview.html` a
  **100/100**; os três conceitos históricos a 90–95/100 (só lhes falta `<title>` e, em dois,
  `width`/`height` fixos na raiz).

---

## 1. Personalidade

Cinco adjectivos, na ordem em que mandam:

| Adjectivo     | O que significa no desenho                                                                 |
| ------------- | ------------------------------------------------------------------------------------------ |
| **Preciso**   | Ângulos exactos (60°, 45°, 90°), espessuras iguais, nada «a olho».                          |
| **Calmo**     | Sem gradientes, sem brilhos, sem sombras. Sem cor de marca: só cinzas.                      |
| **Honesto**   | Mostra o que mede. O logo não promete velocidade nem jogos; promete controlo verificável.    |
| **Técnico**   | Parece um instrumento, não um brinquedo. A referência é a folha de desenho, não o dashboard. |
| **Contido**   | Um símbolo, uma escala, uma regra. Se uma ideia precisa de legenda, não entra.               |

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
a grelha que o mede. A marca **não** usa o accent da UI: é grayscale por decisão (§6).

**Relação com a palavra «control».** Controlar não é ter muitos botões; é ter autoridade sobre um
valor. O símbolo não mostra um painel, mostra o ponto onde a autoridade se exerce. Daí também a
disciplina do resto: uma espessura, uma escala de cinzas, zero cor.

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

O wordmark é **todo `currentColor`**. Não há accent para entrar na tipografia — ver §6.

## 6. Cor — só cinzas

A identidade não tem cor de marca. A paleta é uma escala neutra: quatro tokens de tinta/placa e os
dois planos do tema. A única «cor» que o logo usa é a diferença entre tinta e placa.

| Token         | Hex       | Onde                                        | Croma  |
| ------------- | --------- | ------------------------------------------- | ------ |
| **tinta**     | `#191B1D` | símbolo, wordmark, glifo no tile claro      | 4/255  |
| **placa clara** | `#E8E9EA` | tile no tema claro; ícone da app          | 2/255  |
| **placa escura** | `#232527` | tile em superfícies escuras              | 4/255  |
| **tinta clara** | `#F2F3F4` | M sobre a placa escura                     | 2/255  |
| plano claro   | `#F3F5F6` | fundo do tema claro (`--bg`)                | 3/255  |
| plano escuro  | `#15181A` | fundo do tema escuro                        | 2/255  |

**Nenhuma destas cores tem matiz.** O croma máximo é 4/255 — abaixo do limiar em que o olho lê «cor».
O antigo accent periwinkle `#87A2EA` (croma 99/255) foi retirado na v1.1 a pedido do dono e **não teve
substituto**: a marca é contraste, não cor. Não há gradiente, sombra nem opacidade em nenhum ficheiro.

**Contrastes medidos (WCAG 2.x, luminância relativa):**

| Par                              | Rácio   | Leitura                                                        |
| -------------------------------- | ------- | -------------------------------------------------------------- |
| tinta `#191B1D` sobre placa `#E8E9EA` | 14,21:1 | **AAA** — o glifo do ícone (AA pede 4,5:1)                |
| tinta clara `#F2F3F4` sobre placa `#232527` | 13,84:1 | **AAA** — o glifo no tile escuro                    |
| tinta sobre branco               | 17,27:1 | AAA — símbolo e wordmark no tema claro                        |
| placa clara sobre branco         | 1,22:1  | só o bordo da placa; o M escuro é que carrega a forma         |
| placa escura sobre o plano escuro | 1,16:1 | a placa dissolve-se; o M claro carrega a forma                |
| placa escura sobre branco        | 15,38:1 | a placa salta no claro                                        |

**O tile de app é o claro** (`logo-tile.svg`: `#E8E9EA` + `#191B1D`, 14,21:1). É a variante que se lê
nas duas barras do Windows: na escura a placa é o objecto claro; na clara o M escuro é a forma. O
bordo da placa sobre branco é apenas 1,22:1 — não é contraste de leitura e não se vende como tal; o
que garante a leitura é o glifo. **O tile escuro** (`logo-tile-dark.svg`: `#232527` + `#F2F3F4`,
13,84:1) existe para superfícies escuras dentro da app; aí a placa quase desaparece (1,16:1) e o que
fica é o M claro — é intencional.

**Regra:** a paleta é fechada. Qualquer cor com croma > 6/255 é defeito e o `svg_audit.py` chumba o
ficheiro (foi assim que o roxo antigo foi varrido de todos os SVGs). Se a UI precisa de estado, usa o
accent dela; a marca não empresta cor a botões, gráficos ou texto.

## 7. Ficheiros e como se regeneram

```
docs/brand/
├─ BRAND.md                     este documento (v1.1 — grayscale)
├─ preview.html                 folha de aprovação — self-contained: SVG inline + PNG em data: URI
├─ logo-mark.svg                símbolo (M, currentColor)                 ← mestre
├─ logo-mark-small.svg          corte pequeno (haste 22%, currentColor)   ← 16–24 px mono
├─ logo-tile.svg                ícone da app (placa #E8E9EA + M #191B1D)  ← mestre do ícone
├─ logo-tile-dark.svg           ícone para superfícies escuras (#232527 + #F2F3F4)
├─ logo-wordmark.svg            «MiControl» (currentColor)
├─ logo-primary.svg             lockup horizontal (placa clara + wordmark)
├─ logo-primary-dark.svg        lockup horizontal para fundo escuro
├─ logo-primary-mono.svg        lockup a uma tinta (placa com M vazado, evenodd)
├─ logo-stacked.svg             lockup vertical
├─ logo-stacked-dark.svg        lockup vertical para fundo escuro
├─ concepts/                    as três direcções exploradas (A, B, C), a preto
├─ icons/                       icon set pronto a copiar para src-tauri/icons
└─ tools/
   ├─ build_brand.py            geometria → SVG (stdlib, determinístico)
   ├─ rasterize.py              SVG → PNG/ICO (Chrome headless em lote)
   ├─ build_preview.py          SVG → preview.html (inline + data: URI) e valida o self-containment
   └─ svg_audit.py              auditoria: vectorial puro, paleta cinza, AA, preview sem referências
```

```bash
python docs/brand/tools/build_brand.py     # reescreve os 10 SVG
python docs/brand/tools/rasterize.py       # reescreve icons/ (PNG + ICO + tray)
python docs/brand/tools/build_preview.py   # reescreve preview.html e falha se houver referência externa
python docs/brand/tools/svg_audit.py       # auditoria (exit 1 se algum ficheiro < 90)
```

**`preview.html` é self-contained desde a v1.1.** Na v1.0 usava `<img src="icons/…">` e
`<img src="logo-…svg">` relativos: no repo abria bem, mas num widget embutido (iframe/outro contexto)
as imagens apareciam quebradas. Agora os SVG entram como markup inline e os PNG como `data: URI`
base64; o `build_preview.py` tem um validador que falha o build se existir um `src`/`href` que não
seja `data:`, um `<link>`, um `@import` ou um `url()` externo.

Os quatro correm com o Python do sistema, sem dependências externas além do Pillow (que já existe) e de
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
5. **O `src-tauri/icons/icon.icns` foi regenerado** a partir de um render de 1024 px da placa
   (Pillow, entradas 16/32/128/256/512 + @2x), porque nesta máquina não há `iconutil`. O produto só
   empacota `nsis` (Windows), por isso o `.icns` não entra em nenhum bundle actual; se houver macOS,
   vale reconfirmar com `iconutil` antes do primeiro build.

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

**Fundos aprovados:** `--bg` claro, `--surface` claro, `--bg` escuro, `--surface` escuro, placa clara
(no claro) e placa escura (no escuro). Sobre fotografia, usar sempre a versão mono com o M vazado.

**Nunca:**

- pôr o M solto ao lado de «MiControl» — lê-se «MMiControl» (aconteceu no primeiro render deste
  trabalho); na sidebar o M vive na linha do logo, com o texto à direita;
- usar qualquer cor com matiz (a paleta é fechada; croma > 6/255 é defeito);
- rodar, inclinar, contornar, sombrear ou aplicar gradiente ao símbolo;
- esticar o wordmark (a largura é consequência da construção, não um parâmetro);
- usar o M dentro de um círculo, quadrado ou «badge» que não seja a placa definida;
- recolorir a placa com uma cor que não seja `#E8E9EA`/`#232527` (o ícone perde o contraste medido).

## 10. Aplicado na app

Aplicado no branch `f8/identity` (direcção aprovada pelo dono). Passos executados:

```bash
# 1. ícones da app (mesmos nomes, mesmo sítio)
cp docs/brand/icons/*.png docs/brand/icons/icon.ico src-tauri/icons/

# 2. favicon que o HTML já declara e não existe
mkdir -p public/icons && cp docs/brand/logo-tile.svg public/icons/icon.svg
```

```tsx
// 3. src/components/MiControlIcon.tsx — placeholder substituído pelo símbolo novo.
//    Usa o CORTE PEQUENO (logo-mark-small.svg: haste 22%, sem chanfros), que é o
//    especificado para 16–24 px; o M antigo era um polyline de 2,5 com dois pontos
//    e uma barra em var(--accent).
<path
  fill="currentColor"
  d="M0 0L17 0L39 38.105L61 0L78 0L78 0L78 72L78 72L61 72L61 33.862L39 71.967L17 33.862L17 72L0 72L0 72L0 0Z"
/>
// viewBox="-2 -2 82 76"; height = size × 76/82 (a marca não é quadrada)
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
a geometria e `docs/brand/logo-mark-small.svg` é o resultado — copiar o `d` de lá.

**Nota sobre o favicon:** `index.html` e `landing.html` declaram `/icons/icon.svg` e `public/icons/`
não existia no repo — 404 silencioso em dev e na landing. O passo 2 criou o ficheiro; os dois builds
(`build` e `build:landing`) copiam-no para `dist/icons/icon.svg` e `dist-landing/icons/icon.svg`.

## 11. O que fica por decidir

| #   | Decisão                                                        | Recomendação                                                              |
| --- | -------------------------------------------------------------- | ------------------------------------------------------------------------- |
| 1   | Aprovar a direcção B (M de precisão)                            | aprovada e aplicada em `f8/identity`                                      |
| 2   | Sidebar: marca a 16 px (recomendado) ou manter 22 px            | **16 px, aplicado** — a 22 px o M fica 1,6× a altura do texto e domina a linha |
| 3   | Tray: placa clara (recomendado) ou M mono                       | **placa clara** — na barra escura a placa é o objecto; na clara o M escuro (17:1 sobre o plano) carrega a forma. Um M mono desaparece numa das duas |
| 4   | Substituir `MiControlIcon` e aplicar os ícones                  | **feito** — ver §10                                                       |
| 5   | Versão empilhada para o README/landing                          | `logo-stacked.svg` já existe; escolher onde entra                          |
| 6   | Accent da UI (`src/styles/globals.css`) ainda é periwinkle      | fora do âmbito da marca; se o dono quiser, neutraliza-se numa passagem só da UI |
| 7   | Registo de marca                                                | **não foi feito nenhum estudo de anterioridade** — recomenda-se pesquisa profissional antes de uso público |

## 12. Limites deste trabalho

- **Não houve pesquisa de anterioridade nem verificação de marcas.** O M é um desenho original, mas
  originalidade não é o mesmo que disponibilidade registral.
- **A auditoria `svg_audit.py` é heurística** (100/100 não é um certificado: é a ausência de falhas
  que a ferramenta sabe detectar — vectorial puro, paleta cinza, viewBox, acessibilidade e o
  contraste do tile).
- **O preview foi verificado em Chrome/Edge headless a partir de `file://` no repo e a partir de uma
  cópia em `%TEMP%`** — o segundo teste simula o caso embutido (sem os ficheiros vizinhos). Os ícones
  aparecem nos dois.
- **Os ícones foram vistos a 16, 24, 32, 48, 64, 128, 256 e 512 px** em fundo claro e escuro, mas não
  foram instalados numa barra de tarefas real do Windows 11 com DPI 125/150%. Vale a pena confirmar
  visualmente depois do passo §10.
- **`icon.icns` não foi regenerado** (sem ferramentas de macOS nesta máquina) e `android/`/`ios/` do
  Tauri ficaram intocados — este produto é Windows-only.