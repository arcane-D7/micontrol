# Code signing Authenticode — opções de custo zero/baixo (estado em novembro de 2026)

> **Pergunta original:** “Não há nenhum sistema gratuito de certificados? Open Source ou qualquer outro... investigue tudo, até gray web!”
>
> **Resposta curta e honesta:** **não existe** um certificado Authenticode gratuito emitido **em nome do autor** que seja aceite pelo Windows e que limpe o SmartScreen. Existem **dois** caminhos gratuitos reais, ambos com um custo que não é dinheiro:
>
> 1. **SignPath Foundation** — assina de graça, mas o *publisher* que o utilizador vê é **“SignPath Foundation”**, não “miControl” nem o teu nome.
> 2. **Microsoft Store com MSIX** — a Microsoft re-assina o pacote, o publisher é a Microsoft e **não há aviso nenhum**; mas é outro canal de distribuição, não o instalador NSIS do GitHub.
>
> Tudo o resto é pago (€49–€600/ano), ou não é Authenticode (Sigstore/Let’s Encrypt/attestations), ou é ilegal/arriscado (certificados roubados, revenda cinzenta, “bypasses”).
>
> Para o caso concreto do miControl (1 dev, app pequena, GitHub Releases, NSIS), a recomendação é: **candidatar já ao SignPath Foundation** e, em paralelo, decidir se aceitas o publisher “SignPath Foundation”; se não aceitares, **Certum Open Source a €49** é o único caminho legítimo barato com o teu nome.

Documento preparado em **2026-11-14**. Todos os preços e regras foram verificados nas fontes citadas na secção 12; preços de CAs excluem IVA salvo indicação em contrário (IVA PT = 23%).

---

## 1. Contexto do projecto (factos verificados no repo)

| Item | Estado |
|---|---|
| Repositório | `github.com/arcane-D7/micontrol` — **público**, `visibility: PUBLIC` (verificado com `gh repo view`) |
| Licença | **MIT** (OSI-approved, sem dupla licença comercial) — requisito cumprido para programas OSS |
| Releases | Sim: `v0.2.20` (2026-10-25), `v0.1.25`, `v0.1.24`, … — requisito “já lançado” cumprido |
| Stack | Tauri v2, `bundle.targets: ["nsis"]` — **só gera EXE NSIS**, não MSI nem MSIX |
| Updater | Tauri updater com chave **minisign** (`pubkey` em `tauri.conf.json`) — assinatura de *payload*, **não** é Authenticode |
| CI | `release.yml` constrói o shell **sem assinatura** (`signing state: unsigned (expected for CI shell; local publisher signs with cert)`) |
| Assinatura actual | `scripts/publish-release.ps1` assina localmente com `signtool /sha1 <thumbprint> /fd SHA256 /tr http://timestamp.digicert.com` |
| Popularidade | 5 stars, 0 forks — relevante para reputação SmartScreen (ver §6) |

**Consequência prática:** o binário que o utilizador descarrega do GitHub não tem assinatura Authenticode válida em nome de ninguém. O UAC/SmartScreen mostram “Unknown publisher”. Para uma app que gere hardware e provavelmente pede elevação, isto é o problema mais visível.

---

## 2. O que mudou em 2026 (regras que afectam qualquer opção)

1. **Validade máxima de 460 dias** para certificados de code signing publicamente confiáveis, desde **1 de Março de 2026** (CA/Browser Forum Ballot CSC-31, BR v3.10.0). Acabaram os certificados de 3 anos; renova-se ~1×/ano. As CAs emitem a 458–459 dias por margem. Fonte: <https://cabforum.org/2025/11/17/ballot-csc-31-maximum-validity-reduction> e <https://cabforum.org/working-groups/code-signing/requirements/>
2. **Chave privada obrigatoriamente em hardware** (token USB ou HSM cloud) desde Junho de 2023 — sem excepção para indivíduos. Fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>
3. **EV já não dá reputação instantânea no SmartScreen** desde 2024. Pagar EV só para “saltar” o aviso é dinheiro deitado fora. Fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>
4. **Timestamp é obrigatório na prática**: sem timestamp RFC 3161, a assinatura morre quando o certificado expira (aos ~15 meses) e o Windows trata o ficheiro como não assinado.

---

## 3. Comparação rápida

| Opção | Custo | Publisher que o utilizador vê | SmartScreen 1.º download | Elegível para ti? | Esforço |
|---|---|---|---|---|---|
| **SignPath Foundation** | **€0** | `SignPath Foundation` | Aviso + nome do publisher; reputação acumula | **Sim** (MIT, público, released) | Médio (política + CI + aprovação manual) |
| **Microsoft Store (MSIX)** | **€0** | Microsoft (re-assina) | **Sem aviso** | Sim, mas é outro canal | Alto (MSIX manual, sem updater próprio) |
| **Certum Open Source** | **€49** + IVA (cloud) / €69 set | `Open Source Developer, <o teu nome>` | Aviso + nome; reputação acumula | **Sim** (projecto público, não comercial) | Baixo/Médio (~1 dia + 1–5 dias emissão) |
| **Azure Artifact Signing** | $9,99/mês | A tua identidade validada | Aviso; reputação acumula | **Não** como indivíduo em PT; só empresa UE com **3+ anos** de histórico fiscal | Médio |
| **OV/IV de CA** | $129–€600/ano | O teu nome/empresa | Aviso; reputação acumula | Sim | Médio |
| **OSSign** | €0 | `OSSign` | Aviso | Sim, mas **candidaturas suspensas** | Médio |
| **Self-signed** | €0 | Nome que tu escreveres (não verificado) | **Aviso igual a não assinado** | Só interno | Baixo |
| **Sem assinatura** | €0 | “Unknown publisher” | Aviso forte; empresas podem bloquear | — | — |
| **Certificados “cinzentos”/roubados** | $295–$6.500 | Empresa alheia | Pode passar… até ser revogado | **Ilegal** | — |

---

## 4. Opção 1 — SignPath Foundation (a melhor gratuita para OSS)

**O que é:** a SignPath.io (empresa) doa o serviço de assinatura à SignPath Foundation (fundação). A fundação tem o seu próprio certificado **OV-level**, com a chave num HSM, e assina binários de projectos open source que cumpram os critérios. A Microsoft refere explicitamente esta opção na documentação oficial: *“If your project is open source, SignPath Foundation offers free code signing for qualifying open-source projects. The program provides OV-level certificate signing through a managed pipeline.”* Fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>

### 4.1 Requisitos (todos verificados nos termos)

Fonte: <https://signpath.org/terms.html>

- Licença **OSI-approved**, **sem dupla licença comercial** em nenhum componente.
- **Sem código proprietário** (nem de mantenedores ou afiliados). Bibliotecas de sistema são aceites.
- Projecto **activamente mantido** e **já lançado** na forma que será assinada.
- Funcionalidade **documentada** na página de download ou loja.
- Repositório **público** e build **automatizado** — a assinatura é feita via SignPath.io, com verificação de origem (o artefacto tem de vir do build do repo).
- **MFA** para todos os membros da equipa.
- Publicar uma **“Code Signing Policy”** no README/página de download com:
  - a frase obrigatória: *“Free code signing provided by SignPath.io, certificate by SignPath Foundation”*;
  - papéis e membros (committers/reviewers, approvers).
- Metadados **product name** e **product version** definidos e impostos na configuração de artefacto (o Tauri já os injecta no NSIS).
- **Aprovação manual de cada pedido de assinatura** no dashboard (mais um passo humano por release).
- Só podes assinar **os teus próprios binários**, gerados do teu próprio código.

### 4.2 O que podes assinar

EXE, MSI, DLL, JAR, XML, macros Office, imagens Docker, e **artefactos aninhados** (ex.: instalador que contém DLLs). O instalador **NSIS do Tauri é um EXE** — assina sem problema. Fonte: <https://signpath.io/solutions/open-source-community> e <https://github.com/sebbourgeois/promptbox/blob/master/docs/SIGNPATH.md>

### 4.3 Custo real

- **Dinheiro: €0.** Sem token, sem shipping, sem renovação.
- **Tempo:** aplicação + preparação da política + integração no GitHub Actions + aprovação manual em cada release. Estimativa realista: **meio dia a dois dias** de trabalho de integração.
- **Prazo até ter certificado:** **não há SLA publicado**. Há relatos de aceitação em semanas e de meses até a emissão do certificado de produção. Exemplo documentado: o projecto Pulse foi aceite em **2026-08-06** e, na última actualização da sua política, o certificado de produção ainda estava `CSR PENDING` — ou seja, **meses** entre aceitação e assinatura real. Fonte: <https://github.com/rcourtman/Pulse/blob/main/docs/CODE_SIGNING_POLICY.md>
- **Risco:** a fundação reserva-se o direito de **suspender ou revogar** (inclusive retroactivamente) se violares as regras. Uma revogação atinge **todos** os projectos que usam aquele certificado — é um risco partilhado.

### 4.4 A grande desvantagem (decisão política, não técnica)

O certificado é emitido **à SignPath Foundation**. Isso significa:

- No UAC e nas propriedades do ficheiro aparece **“SignPath Foundation”**, não “miControl” nem “Diogo …”.
- A reputação SmartScreen acumula-se na **identidade SignPath Foundation**, partilhada por centenas de projectos. É bom (reputação inicial melhor) e mau (se outro projecto abusar, todos sofrem).
- Para um app de hardware com alguma identidade de marca, isto pode ser aceitável ou inaceitável — é uma escolha tua, não uma limitação técnica.

### 4.5 Como te candidatas (passos concretos)

1. Antes de tudo, cria `docs/CODE_SIGNING_POLICY.md` (ou secção no README) com a atribuição obrigatória, papéis e âmbito.
2. Garante `LICENSE` na raiz (já tens, MIT), README com descrição e link de download, build via GitHub Actions público (já tens).
3. Preenche o formulário em **<https://signpath.org/apply.html>** com: URL do repo, licença, URL de releases/download, descrição do projecto e dos artefactos (EXE NSIS).
4. Depois de aceite: instalar a GitHub App da SignPath, criar organização/projecto, configurar *artifact configuration* (metadata restrictions), criar *signing policy* com o teu utilizador como approver, e adicionar o step `signpath/github-action-submit-signing-request` ao `release.yml`.
5. Cada release: o CI submete o instalador, tu aprovas no dashboard, o artefacto assinado volta ao workflow.

### 4.6 Adequação ao miControl

**Boa.** Cumpre todos os requisitos formais. O único blocker é conceptual: o publisher passa a ser a fundação. Se o objectivo é “não ter ‘Unknown publisher’ e ter integridade verificável”, resolve. Se o objectivo é “marca MiControl visível no UAC”, não resolve.

---

## 5. Opção 2 — Certum Open Source Code Signing (o mais barato com o teu nome)

**Ainda existe em 2026?** Sim. É um produto real da Asseco Data Systems (Certum, Polónia), CA pública membro do CA/Browser Forum, com certificado **real** (não self-signed), chain para a raiz Certum que está no Microsoft Trusted Root Program. Fonte oficial: <https://www.certum.eu/en/code-signing-certificates> e <https://shop.certum.eu/open-source-code-signing-on-simplysign.html>

### 5.1 Preços (loja oficial, Nov 2026)

| Variante | Preço | Chave privada | Notas |
|---|---|---|---|
| **In the Cloud (SimplySign)** | **desde €49** (net) | HSM cloud da Certum (FIPS 140-2 L3 / CC EAL 4+) | Sem hardware; app móvel + SimplySign Desktop |
| **Set (cartão + leitor)** | **desde €69** | Cartão criptográfico cryptoCertum (CC EAL6+) | Envio físico; leitor incluído; shipping extra (~€35 para UK no relato de 2025) |
| **Só código de activação** | **desde €25** | O teu cartão cryptoCertum 3.6/3.7 | Só se já tens cartão + leitor |

Com IVA PT (23%): o cloud fica ~**€60,27**. Validade: **1 ano** (o certificado em si ≤460 dias por CSC-31).

### 5.2 Requisitos e limitações (importantes)

Fonte: <https://support.certum.eu/en/code-signing-required-documents/>

- **Só para pessoas singulares** — organizações não podem ter este produto.
- O certificado contém o prefixo fixo **“Open Source Developer”** no Common Name e `O=Open Source Developer`; o teu nome aparece a seguir.
- **Proibido uso comercial**: *“If Certum determines that the certificate is being used to sign software distributed commercially, the certificate will be revoked.”*
- Verificação de identidade: ID automático (IDNow), presencial, notário ou fotos com documento; **mais uma factura de serviços** (água/luz/gás/telefone) emitida em teu nome.
- **Website do projecto open source, público, que mostre claramente a tua relação com o projecto**. Se a Certum não conseguir identificar o projecto, **não emite**.
- Não pode conter domínios nem IPs no certificado.
- **Limite de 5.000 assinaturas/mês** na variante cloud; exceder pode bloquear o produto no mês.
- Emissão típica: **1–5 dias úteis** após documentos completos.
- Chave RSA ≥3072 bits.

### 5.3 Experiência real de utilização (relato first-hand, Out 2025)

Fonte: <https://piers.rocks/2025/10/30/certum-open-source-code-sign.html>

- Compra do set (€69) + shipping; cartão e leitor chegam em ~2 dias.
- Instalação de `proCertum CardManager`, drivers do leitor (o link da Certum não aponta para o driver certo — tive de procurar), reboot, configuração de PIN/PUK.
- Verificação de identidade via IDNow no telemóvel (cartão de condução + prova de vida), upload de factura da água e de um PDF com links para o site/repo/licença.
- Certum demorou **2 dias** a aprovar; depois foi preciso instalar o **SignService** (diferente de SimplySign) para activar o certificado no cartão.
- **A parte difícil foi o signtool**: o certificado aparece em `certmgr.msc` mas com “No key provider information / Missing stored keyset”; o signtool só funcionou depois de associar correctamente a chave do cartão. **“All told I probably spent about a day on this.”**
- Renovação seguinte: ~€29 (já tens cartão/leitor).

### 5.4 Variante cloud vs cartão — qual escolher

- **Cloud (€49)**: sem hardware, mas exige **SimplySign Desktop** em execução e aprovação na app móvel. **Automatizar em GitHub-hosted runners é doloroso** (sessão interactiva, app móvel). Na prática: assinas localmente antes de publicar, ou num self-hosted runner. Se o teu fluxo é “CI constrói, eu assino no PC antes de publicar” — que é o que o `publish-release.ps1` já faz — o cloud serve.
- **Cartão (€69)**: mais chato de montar (drivers, PIN, signtool), mas fica tudo local e reprodutível. Para um único dev num PC Windows, é a opção mais estável a longo prazo.

### 5.5 Adequação ao miControl

**Muito boa, se o projecto for e continuar não comercial.** É o único caminho barato que põe o **teu nome** no UAC. Dois riscos a ter em conta:

1. Se algum dia venderes o miControl, ou o distribuirem como parte de um produto pago, a Certum **revoga** — e uma revogação é pior que não ter certificado (assinaturas passam a “revoked”).
2. Disponibilidade: em Out 2026 vários produtos OSS da Certum apareciam **“out of stock”** em listagens de terceiros; a página oficial hoje mostra “Add to Cart”. Confirma antes de contar com isso.

---

## 6. Opção 3 — Azure Artifact Signing (ex-Trusted Signing)

**Preço actual (confirmado):**

| SKU | Preço/mês | Assinaturas incluídas | Excedente | Perfis |
|---|---|---|---|---|
| **Basic** | **$9,99** | 5.000 | $0,005/assinatura | 1 de cada tipo |
| **Premium** | **$99,99** | 100.000 | $0,005/assinatura | 10 de cada tipo |

Fontes: <https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-change-sku> e <https://azure.microsoft.com/en-us/products/artifact-signing>

### 6.1 Elegibilidade — aqui é que a opção morre para ti

Fonte oficial: <https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview> e <https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart>

- **Public Trust para organizações:** EUA, Canadá, **União Europeia**, Reino Unido, Austrália, Nova Zelândia, Japão, Coreia do Sul, Singapura, Suíça, Noruega, Israel.
- **Public Trust para indivíduos: apenas EUA e Canadá.** Portugal **não** está incluído.
- **Organizações precisam de “verifiable tax history of three or more years”.** Confirmado na documentação de assinatura MSIX e em respostas Microsoft Q&A sobre GA: uma LLC criada em 2025 foi recusada por não ter 3 anos.
- É preciso **subscrição Azure paga** (pay-as-you-go/enterprise); a documentação e relatos indicam que subscrições free/trial/sponsored são recusadas.
- Validação de identidade demora “alguns dias úteis”; o certificado é de curta duração (emitido diariamente, válido poucos dias) e o timestamp mantém a assinatura válida.

### 6.2 O que isto significa na prática

| Cenário | Elegível? |
|---|---|
| Tu, indivíduo, residente em Portugal | **Não** |
| Empresa portuguesa constituída em 2024/2025/2026 | **Não** (falta histórico fiscal de 3 anos) |
| Empresa portuguesa com 3+ anos de actividade e registo fiscal | **Sim** (≈$120/ano) |
| Membro de equipa/empresa dos EUA ou Canadá | Sim, como indivíduo |

### 6.3 Integração (se algum dia fores elegível)

- `winget install -e --id Microsoft.Azure.ArtifactSigningClientTools` (o SignTool normal **não** funciona sozinho; precisa do plugin dlib + .NET 8 + `metadata.json`).
- GitHub Actions: `azure/trusted-signing-action` com OIDC federado (sem segredos de longa duração).
- No Tauri: assinar o `*-setup.exe` depois do build, antes do upload.

**Veredicto:** tecnicamente excelente, economicamente imbatível — **inacessível** para um indivíduo em Portugal. Só faz sentido se um dia tiveres uma empresa com 3+ anos. Não é um atalho.

---

## 7. Opção 4 — Microsoft Store / MSIX (a única forma gratuita de **não ter aviso nenhum**)

Esta é a descoberta mais importante da investigação, e a que a maioria dos guias de Tauri não destaca.

### 7.1 Factos oficiais

- **Conta de developer individual: gratuita.** A taxa de $19 foi eliminada no novo fluxo; entra-se por **<https://storedeveloper.microsoft.com>** (qualquer outro caminho — Partner Center directo, Xbox, Visual Studio — mostra o fluxo antigo e pode pedir pagamento). Verificação com documento de identidade + selfie. Fonte: <https://learn.microsoft.com/en-us/windows/apps/publish/whats-new-individual-developer>
- **Conta de empresa: também sem taxa de registo** no fluxo actual (a página oficial diz “no registration fees for either account type”).
- **MSIX/AppX submetidos à Store não precisam de certificado**: *“the package does not need to be signed with a CA-trusted certificate… the Microsoft Store automatically re-signs your package with a Microsoft certificate”*. Fontes: <https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements> e <https://learn.microsoft.com/en-us/windows/apps/publish/get-started>
- **MSI/EXE submetidos à Store NÃO são re-assinados.** Têm de ser assinados por ti com um certificado que encadeie para o Microsoft Trusted Root Program; **self-signed não é aceite**. Fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>
- **SmartScreen:** apps instaladas via Store **nunca** mostram aviso de download; têm “full reputation”. Fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>
- **Política 10.2.5:** a app não pode actualizar-se fora da Store. O updater do Tauri tem de ser **desactivado** na variante Store.

### 7.2 O problema do Tauri

O Tauri v2 **não gera MSIX**. A documentação oficial diz: *“Currently Tauri only generates EXE and MSI installers, so you must create a Microsoft Store application that only links to the unpacked application.”* Fonte: <https://v2.tauri.app/distribute/microsoft-store/>

Ou seja, para MSIX tens de:

1. Construir a app (`tauri build --no-bundle` ou build normal).
2. Escrever um `Package.appxmanifest` (identity, `runFullTrust`, `internetClient`, assets/logos, versão `x.y.z.0`).
3. Correr `MakeAppx pack /d staging /p MiControl_x.y.z.0_x64.msix` (Windows SDK, gratuito).
4. Passar o **Windows App Certification Kit** (`appcert.exe`).
5. Submeter no Partner Center (revisão típica: até 3 dias úteis).
6. **Desactivar o updater próprio** na build Store (o Tauri tem de ser compilado com o updater desligado; a Store faz os updates).

Ferramentas que ajudam: `winapp` CLI da Microsoft (`winapp init` gera manifest + assets para Tauri — <https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli/guides/tauri>) e o pacote comunitário `@choochmeque/tauri-windows-bundle` (gera MSIX/MSIXBUNDLE a partir do `tauri.conf.json`).

### 7.3 Custos e trade-offs

| Item | Custo/Impacto |
|---|---|
| Conta Store individual | €0 |
| Certificado | €0 (Microsoft re-assina) |
| SmartScreen | **Zero avisos** |
| Hosting/updates | Microsoft |
| Trabalho de packaging | 1–3 dias (manifest, assets, WACK, certificação) |
| Perda do updater Tauri/minisign | Sim — updates passam a ser da Store |
| Descoberta | Ganhas loja; perdes o fluxo “descarrega o .exe do GitHub” |
| Modelo de receita | Se venderes, a Store tem regras próprias; para apps não-jogo podes usar comércio próprio (0% Microsoft) ou o da Microsoft (15%) |

### 7.4 Veredicto

**É a única via legítima, gratuita e sem avisos.** Não substitui o instalador NSIS do GitHub — é uma **segunda** via de distribuição. Para uma app de hardware que pede admin e cujo público desconfia de instaladores, ter “também está na Microsoft Store” é um argumento de confiança forte.

Nota: MSIX **fora** da Store (sideload) volta a exigir assinatura própria — o benefício só existe dentro da Store.

---

## 8. Opção 5 — GitHub: o que existe e o que não existe

| Coisa | É Authenticode? | Serve para quê |
|---|---|---|
| **GitHub Artifact Attestations** (`actions/attest-build-provenance`) | **Não.** É Sigstore (Fulcio/Rekor). Gera um bundle JSON com proveniência; o `.exe` **não** ganha assinatura PE embutida. | Provar que o binário saiu daquele workflow. Verificação: `gh attestation verify`. Grátis em repos públicos. Fonte: <https://docs.github.com/en/actions/concepts/security/artifact-attestations> |
| **GitHub Actions + OIDC → Azure Artifact Signing** | **Sim** (a assinatura é feita pelo Azure, não pelo GitHub) | Autenticação sem segredos ao serviço da Microsoft. Fonte: `azure/trusted-signing-action` |
| **GitHub como CA / certificado de code signing** | **Não existe.** | — |
| **GitHub Releases + minisign (Tauri updater)** | **Não.** Assina o payload do updater. | Protege o canal de update; não limpa o primeiro download. |

**Conclusão:** o GitHub dá-te o *plumbing* (CI, OIDC, attestations, releases), não o certificado. As attestations são úteis como camada extra de proveniência — e podes adicioná-las já, a custo zero — mas não mudam nada no SmartScreen nem no UAC.

---

## 9. Opção 6 — OSSign (a “outra” SignPath)

- **O que é:** iniciativa sueca (Scheibling Consulting AB), com certificado próprio, que assina binários de projectos OSS. Assina projectos reais (ex.: LibreWolf/WinUpdater, USB/IP client). Código e CLI em <https://github.com/OSSign/OSSign>; site <https://ossign.org>.
- **Custo:** €0.
- **Requisitos:** licença OSI na maioria do código, repo público, **≥6 meses de actividade** e alguma base de utilizadores, **pipeline de build público e verificável**, sem fins comerciais/políticos.
- **Estado em Nov 2026:** **candidaturas suspensas** — *“Applications are currently suspended due to a high workload and large backlog”*. Fonte: <https://ossign.org> (consultado 2026-11-14).
- **Publisher:** “OSSign” (eles), não tu.
- **Risco:** organização pequena, recente (2025), um único maintainer principal; depende dela para renovar e manter o certificado. Não é uma CA — é um serviço de assinatura.

**Veredicto:** alternativa de recurso ao SignPath, hoje **fechada**. Vale a pena monitorizar; não vale a pena planear em cima dela.

### 9.1 “EVSigning — free EV code signing for open source”

Encontrei <https://evsigning.com/free-code-signing>, que anuncia “EV Code Signing gratuito para OSS, tempo limitado”, com badge e Discord. **Não consegui verificar quem está por trás, que CA emite, nem um único projecto assinado.** Bandeiras vermelhas clássicas: “limited-time”, escassez artificial, badge de “verified”, ausência de entidade legal identificável. **Não recomendo.** Se algum dia considerares, exige: nome da CA emissora, número de série de um certificado já emitido, e verifica a cadeia no Windows antes de lhes entregar o teu binário.

---

## 10. Opção 7 — Self-signed e reputação sem certificado

### 10.1 Self-signed com raiz instalada (distribuição interna)

**Dá para distribuição interna? Sim.** Mas com condições:

- O certificado tem de ser instalado em **Trusted Root Certification Authorities** *e* **Trusted Publishers** nas máquinas dos utilizadores (via GPO/Intune, ou manualmente).
- Com a raiz confiada, o UAC mostra o nome que puseste no certificado e o instalador corre sem “Unknown publisher”.
- **SmartScreen continua a poder avisar** — a Microsoft trata self-signed como “same behavior as no signature” para o público geral; a confiança na raiz resolve a validação da assinatura, não a reputação.
- Para o público em geral (descarregam do GitHub), **não serve**: ninguém vai instalar a tua raiz.

**Casos válidos:** testes, beta fechado, empresas com IT que gere as máquinas, distribuição interna numa organização.

**Como gerar (exemplo):**

```powershell
$cert = New-SelfSignedCertificate -Type CodeSigningCert `
  -Subject "CN=MiControl Internal" -KeyUsage DigitalSignature `
  -CertStoreLocation "Cert:\CurrentUser\My" -NotAfter (Get-Date).AddYears(1)
Export-PfxCertificate -Cert $cert -FilePath .\micontrol-internal.pfx -Password (Read-Host -AsSecureString)
signtool sign /f .\micontrol-internal.pfx /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 .\MiControl_x.y.z_x64-setup.exe
```

### 10.2 Reputação SmartScreen sem certificado

Factos da Microsoft (fonte: <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>):

- SmartScreen avalia **duas** coisas: reputação do **publisher** (assinatura) e reputação do **hash** do ficheiro.
- Sem assinatura, **cada versão começa do zero**. A reputação **não transita** entre versões.
- Com certificado, a reputação pode acumular na **identidade do publisher** e transitar para versões novas assinadas com o mesmo certificado.
- **Não há limiar publicado.** A Microsoft diz “several weeks and hundreds of clean installs from a wide audience”.
- **Não há forma de submeter para “whitelist”** em endpoints de consumidor. Submissões no WDSI servem para falsos positivos de malware, não para reputação.
- **EV já não acelera nada** (desde 2024).
- **Mudar de certificado** (ou de nome de publisher) **reseta** o sinal de confiança.

**Realidade do miControl:** 5 stars e poucos downloads. Sem certificado, o aviso vai aparecer em **todas** as versões, para **todos** os utilizadores, durante muito tempo. Com certificado, aparece na mesma no início — mas com o teu nome em vez de “Unknown publisher”, e a tendência é melhorar. **O certificado não é um botão “remover aviso”; é um botão “mostrar quem sou e começar a acumular reputação”.**

---

## 11. Opção 8 — Alternativas legais que **não** são Authenticode

| Tecnologia | Emite certificado de code signing? | Serve para Windows Authenticode? |
|---|---|---|
| **Let’s Encrypt** | Não. Só DV TLS. FAQ oficial: *“Email encryption and code signing require a different type of certificate that Let’s Encrypt does not issue.”* (<https://letsencrypt.org/docs/faq/>) | **Não** |
| **CAcert** | CA comunitária; não está no Microsoft Trusted Root Program | **Não** (Windows não confia) |
| **Sigstore / cosign / Fulcio** | Certificados efémeros ligados a identidade OIDC; assinatura em transparency log | **Não** — o Windows não lê bundles Sigstore como assinatura PE |
| **OpenSSL self-signed** | Sim, mas self-signed | Só interno, com raiz instalada |
| **GPG** | Assinatura de ficheiros | Não é Authenticode; não afecta SmartScreen |
| **Windows Hardware Dev Center** | Certificados para drivers | Fora do âmbito (não é driver) |

**Conclusão:** não há “Let’s Encrypt do code signing”. A razão é estrutural: um certificado de code signing afirma *identidade*, e a indústria (CA/B Forum + Microsoft Trusted Root) exige validação documental e chave em hardware. Nenhum movimento open source conseguiu (ainda) contornar isso — o SignPath é o mais próximo, e fá-lo assumindo **ele** a identidade.

### 11.1 Financiamento (não é certificado, mas paga-o)

Se o problema for €49–€300/ano, há caminhos de financiamento para OSS: **NLnet / NGI Zero**, **FUTO**, **Sovereign Tech Fund**, **GitHub Sponsors**, **Open Collective**. São competitivos e lentos (meses), e nenhum tem linha orçamental “certificado de code signing” — mas um grant pequeno cobre anos de certificado. Não é uma solução para “esta semana”, é uma solução para “este ano”.

---

## 12. Zona cinzenta / “gray web” — o que é legal, o que é ilegal, e porque não te serve

Esta secção existe porque pediste “até gray web”. Vou ser directo: **as opções baratas fora das CAs não são um atalho; são um risco criminal ou técnico.**

### 12.1 Revendedores “baratos” (€30–€50) — o que são realmente

Há três tipos, e é preciso distingui-los:

**(a) Revenda legítima de um produto de CA.** Ex.: um revendedor autorizado vende Certum/SSL.com/Sectigo com desconto. É legal. O certificado é emitido **no teu nome**, com validação documental, e a chave vai para o teu token/HSM. **Preço realista: $129+ (SSL.com IV), €139+ (Certum Standard), $195+ (Sectigo OV).** Não existe OV/IV legítimo a €30 — a CA tem custos de validação e de auditoria WebTrust.

**(b) “Certificado Open Source” revendido/activado por terceiros.** Alguns sites vendem “code signing para OSS” barato e tratam da activação. Problema: o certificado Certum OSS é **emitido a uma pessoa singular** e a Certum verifica que o projecto é *teu*. Se o vendedor o emite no nome dele, o publisher é ele, não tu; se o emite no teu nome sem seres tu a fazer a verificação, é falsidade na declaração ao CA → **revogação** quando descoberto. Não é um atalho legal.

**(c) “Certificado” que é self-signed com instalação de raiz.** Alguns vendem “code signing certificate” que na realidade é um certificado gerado por eles, acompanhado de instruções para instalar a raiz. Para terceiros, **vale zero** — o Windows não confia. Pior: convencer utilizadores a instalar uma raiz desconhecida é um risco de segurança grave.

### 12.2 Certificados roubados/contrafeitos — ILEGAL

O mercado criminoso existe e está documentado:

- **Recorded Future (Insikt Group):** certificados Authenticode contrafeitos emitidos por Comodo/Thawte/Symantec, registados com **identidades corporativas roubadas**, vendidos a **$295–$1.599**; entrega em 2–4 dias. <https://www.recordedfuture.com/blog/code-signing-certificates>
- **Intrinsec (2024/2026):** mercado de certificados **EV** a **$2.000–$6.500**, com empresas de fachada registadas para o efeito; casos QakBot (SSL.com), Grandoreiro, Poortry. <https://www.intrinsec.com/wp-content/uploads/2026/01/TLP-CLEAR-20240607-The-EV-Signature-Market-EN.pdf>
- **CA/B Forum (2025):** o padrão dominante é “impostor/shell companies”, não roubo; roubos existem mas são raros e usados por actores sofisticados. <https://cabforum.org/2025/10/17/2025-10-17-minutes-of-the-code-signing-certificate-working-group>

**Riscos concretos para quem compra:**

1. **Legal:** usar identidade de outra empresa para obter/emitir um certificado é **falsidade** e **burla**; distribuir software sob identidade alheia é violação de marca e de termos da CA. Em Portugal, pode configurar responsabilidade criminal. Não é “zona cinzenta”, é crime.
2. **Técnico:** as CAs **revogam** rapidamente quando há abuso (aqui e nos relatórios acima). Um binário assinado com certificado revogado é **pior** que um binário não assinado: o Windows mostra “revoked” e muitos AV/EDR tratam como malware.
3. **Reputação:** o certificado roubado já tem reputação **negativa** ou é desconhecido; o SmartScreen e o Defender vão tratar-te como o atacante que usou aquele certificado antes.
4. **Supply chain:** entregar o teu binário a um “signing service” cinzento significa dar-lhe o executável (e, em alguns casos, acesso ao processo de build). Se ele assinar malware com o mesmo certificado, o teu binário herda o flag.
5. **Responsabilidade:** se o teu instalador causar danos e a assinatura apontar para uma empresa que não deste consentimento, a tua posição legal é indefensável.

### 12.3 “Signtool bypass”, patches ao SmartScreen, “desactivar o aviso”

- **Desactivar o SmartScreen na tua máquina** não muda nada para os teus utilizadores. É uma definição local.
- **Patches ao `signtool`/`amsi`/SmartScreen** para “fazer passar” assinaturas: não criam confiança em Windows alheio; muitos são detectados como PUA/malware; e distribuí-los pode violar leis de circunvenção e os termos da Microsoft.
- **Instruir o utilizador a clicar “More info → Run anyway”** é legítimo e é o que a Microsoft recomenda como comunicação honesta — mas não é uma solução, é uma mitigação.
- **“Assinar” com um certificado de outra pessoa** (mesmo com autorização verbal) é o mesmo problema legal do ponto 12.2.

### 12.4 O que é aceitável na “zona cinzenta”

Há uma zona realmente cinzenta **legítima**:

- **Comprar a uma CA menos conhecida mas publicamente confiável** (Certum, HARICA, Actalis, eMudhra) em vez de DigiCert/Sectigo. São CAs auditadas, no Microsoft Trusted Root, e muito mais baratas. **Isto é legal e recomendável.**
- **Usar um serviço de assinatura gerido** (SignPath, OSSign) em vez de comprar certificado. Legal, com o trade-off do publisher.
- **Aproveitar programas OSS/startup** (SignPath, créditos Azure, Microsoft for Startups). Legal, mas com requisitos.
- **Assinar localmente e publicar no GitHub** em vez de assinar em CI. Legal; só perdes automação.

**O que não é aceitável:** certificados sem validação de identidade, certificados emitidos a outra entidade, “serviços” que assinam com o certificado deles sem contrato, e qualquer coisa que envolva identidade roubada.

---

## 13. Tabela final de decisão (para o teu caso)

| Prioridade | Opção | Custo/ano | Tempo até assinar | Publisher no UAC | SmartScreen | Risco |
|---|---|---|---|---|---|---|
| **1. Quero €0 e não me importo com o nome** | SignPath Foundation | €0 | Semanas–meses (sem SLA) | SignPath Foundation | Aviso inicial; reputação partilhada | Revogação da fundação atinge-te |
| **2. Quero €0 e zero avisos** | Microsoft Store (MSIX) | €0 | Dias (packaging) + 3 dias revisão | Microsoft | **Nenhum** | Perdes updater próprio; outro canal |
| **3. Quero o meu nome, barato, não comercial** | Certum Open Source (cloud) | ~€60 c/ IVA | 1–5 dias + ~1 dia de setup | `Open Source Developer, <nome>` | Aviso inicial; acumula | Revogação se comercial |
| **4. Quero o meu nome, comercial** | Certum Standard / SSL.com IV | €139 / $129+token | 1–5 dias | O teu nome/empresa | Aviso inicial; acumula | Nenhum relevante |
| **5. Tenho empresa com 3+ anos** | Azure Artifact Signing | ~$120 | Dias | A tua identidade | Aviso inicial; acumula | Nenhum relevante |
| **6. Interno/testes** | Self-signed + GPO | €0 | Horas | O nome que definires | Aviso (mas UAC ok) | Só serve dentro da organização |
| **7. Ilegal** | Certs roubados/cinzentos | $295+ | Dias | Empresa alheia | Pode passar até revogar | **Criminal + revogação + AV** |

---

## 14. Recomendação concreta (acção)

### Semana 1 — €0, sem compromisso

1. **Criar `docs/CODE_SIGNING_POLICY.md`** com a atribuição obrigatória do SignPath, papéis (authors/reviewers/approvers) e âmbito (instalador NSIS do GitHub Releases).
2. **Candidatar ao SignPath Foundation** em <https://signpath.org/apply.html>. É gratuito e não impede nada; o pior que acontece é “não”.
3. **Adicionar GitHub Artifact Attestations** ao `release.yml` (`actions/attest-build-provenance`). Custo zero, melhora proveniência, independente do certificado.
4. **Continuar a assinar localmente** com o certificado actual (seja qual for) e **garantir timestamp** em todas as assinaturas.

### Semana 2–4 — decisão

- **Se aceitares “SignPath Foundation” como publisher:** esperar pela aprovação, integrar o GitHub Action, configurar aprovação manual. Não compres certificado.
- **Se não aceitares:** comprar **Certum Open Source cloud €49** (ou set €69 se preferires cartão). Preparar: documento de ID, factura de serviços, link público do repo + licença. Assinar localmente no `publish-release.ps1` (já está preparado para `signtool /sha1`).
- **Em paralelo, se quiseres zero avisos:** avaliar MSIX para a Store. É trabalho, mas é a única via gratuita que elimina o aviso por completo.

### O que **não** fazer

- Não comprar certificados em sites sem CA identificável, “sem verificação”, “instantâneo”, ou com preço < $100 para OV/IV.
- Não usar certificados de terceiros, mesmo “emprestados”.
- Não pagar EV só para SmartScreen — já não funciona.
- Não confundir a assinatura minisign do updater com Authenticode: são camadas diferentes e ambas são necessárias.

---

## 15. Erros comuns que este relatório desmonta

| Mito | Realidade |
|---|---|
| “Há certificados de code signing gratuitos” | Só via SignPath/OSSign, e o publisher é deles. Não há certificado gratuito em teu nome. |
| “Let’s Encrypt podia emitir” | Não emite code signing (FAQ oficial). |
| “EV limpa o SmartScreen” | Desde 2024, não. EV = OV para SmartScreen. |
| “Self-signed resolve para os utilizadores” | Só se instalarem a raiz. Para o público, é igual a não assinado. |
| “A Store exige certificado” | MSIX não exige; a Microsoft re-assina. MSI/EXE exige. |
| “O Tauri gera MSIX” | Não. Só NSIS/MSI. MSIX = MakeAppx/manual. |
| “Azure Trusted Signing é para todos” | Indivíduos: só EUA/Canadá. Empresas UE: 3+ anos de histórico fiscal. |
| “Assinar remove o aviso na primeira versão” | Não. O aviso aparece até haver reputação (semanas, centenas de instalações). |
| “O certificado dura 3 anos” | Desde 2026, máximo 460 dias. |
| “Attestations do GitHub são Authenticode” | Não. São Sigstore; o Windows não as lê. |

---

## 16. Fontes (todas consultadas em 2026-11-14)

**Microsoft Learn / Azure**
1. Code signing options for Windows app developers — <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options>
2. SmartScreen reputation for Windows app developers — <https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation>
3. Sign an MSIX package — <https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview>
4. App package requirements for MSIX — <https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements>
5. Get started with Microsoft Store (FAQ code signing) — <https://learn.microsoft.com/en-us/windows/apps/publish/get-started>
6. Quickstart: Set up Artifact Signing — <https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart>
7. Change the account SKU (preços Basic/Premium) — <https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-change-sku>
8. Azure Artifact Signing product page (preços) — <https://azure.microsoft.com/en-us/products/artifact-signing>
9. Azure Artifact Signing pricing — <https://azure.microsoft.com/en-us/pricing/details/artifact-signing/>
10. Free developer registration for individual developers — <https://learn.microsoft.com/en-us/windows/apps/publish/whats-new-individual-developer>
11. Using winapp CLI with Tauri — <https://learn.microsoft.com/en-us/windows/apps/dev-tools/winapp-cli/guides/tauri>
12. Q&A: organização com <3 anos recusada em GA — <https://learn.microsoft.com/en-us/answers/questions/5977141/azure-artifact-signing-trusted-signing-is-a-us-llc>

**SignPath**
13. SignPath Foundation — condições — <https://signpath.org/terms.html>
14. SignPath Foundation — página inicial — <https://signpath.org/>
15. SignPath Foundation — apply — <https://signpath.org/apply.html>
16. SignPath — Open Source Community — <https://signpath.io/solutions/open-source-community>
17. Exemplo de política publicada (Pulse, aceite 2026-08-06, CSR pendente) — <https://github.com/rcourtman/Pulse/blob/main/docs/CODE_SIGNING_POLICY.md>
18. Guia prático de candidatura (Zenn, 2026) — <https://zenn.dev/shm_7ec/articles/signpath-oss-code-signing>
19. Notas de integração (PromptBox) — <https://github.com/sebbourgeois/promptbox/blob/master/docs/SIGNPATH.md>

**Certum**
20. Certum — página de produto code signing — <https://www.certum.eu/en/code-signing-certificates>
21. Certum Shop — Open Source in the Cloud (€49) — <https://shop.certum.eu/open-source-code-signing-on-simplysign.html>
22. Certum Shop — categoria code signing (set €69, code €25) — <https://shop.certum.eu/data-safety/code-signing-certificates.html>
23. Certum — documentos exigidos — <https://support.certum.eu/en/code-signing-required-documents/>
24. Relato first-hand de compra/activação — <https://piers.rocks/2025/10/30/certum-open-source-code-sign.html>

**Tauri / Store**
25. Tauri v2 — Microsoft Store — <https://v2.tauri.app/distribute/microsoft-store/>
26. Tauri issue #8548 (MSIX não suportado) — <https://github.com/tauri-apps/tauri/issues/8548>
27. `@choochmeque/tauri-windows-bundle` (MSIX para Tauri) — <https://github.com/Choochmeque/tauri-windows-bundle>
28. Guia MSIX + Tauri (StoreFast) — <https://storefast.app/guides/publish-tauri-app-to-microsoft-store>

**Normas / indústria**
29. CA/B Forum Ballot CSC-31 (460 dias) — <https://cabforum.org/2025/11/17/ballot-csc-31-maximum-validity-reduction>
30. CA/B Forum Code Signing Baseline Requirements v3.11.0 — <https://cabforum.org/working-groups/code-signing/requirements/>
31. DigiCert — 459-day validity — <https://knowledge.digicert.com/alerts/code-signing-certificates-459-day-validity>
32. Let’s Encrypt FAQ (não emite code signing) — <https://letsencrypt.org/docs/faq/>
33. GitHub Docs — Artifact attestations (Sigstore) — <https://docs.github.com/en/actions/concepts/security/artifact-attestations>

**Mercado cinzento / criminal**
34. Recorded Future — counterfeit code signing certificates — <https://www.recordedfuture.com/blog/code-signing-certificates>
35. Intrinsec — The EV Code Signature Market for eCrime (PDF) — <https://www.intrinsec.com/wp-content/uploads/2026/01/TLP-CLEAR-20240607-The-EV-Signature-Market-EN.pdf>
36. CyberInsider — underground EV market — <https://cyberinsider.com/inside-the-underground-market-for-ev-code-signing-certificates>
37. CA/B Forum — minutes out/2025 (stolen vs imposter) — <https://cabforum.org/2025/10/17/2025-10-17-minutes-of-the-code-signing-certificate-working-group>

**Alternativas OSS**
38. OSSign — <https://ossign.org> (candidaturas suspensas)
39. OSSign — GitHub — <https://github.com/OSSign/OSSign>
40. foropensource — OSSign (verificado 2026-11-10) — <https://foropensource.com/company/ossign/>

**Preços de referência (CAs)**
41. SSL.com — IV Code Signing ($129/ano) — <https://www.ssl.com/products/software-integrity/code-signing/iv/>
42. LeaderSSL — DigiCert OV (€599/ano) — <https://www.leaderssl.com/suppliers/digicert/products/code_signing>
43. My-SSL — Open Source Code Signing em 2026 (contexto e preços) — <https://my-ssl.com/learn/open-source-code-signing-certificate>

---

## 17. Nota final

O miControl está numa posição **invejável** para este problema: repo público, licença MIT, releases regulares, CI a funcionar. Isso qualifica-o para o SignPath Foundation — que é, de longe, a melhor relação custo/benefício — e para o Certum Open Source a €49, que é a única forma barata de ter o **teu** nome no UAC.

O que **não** existe é um certificado Authenticode gratuito, emitido em teu nome, que funcione para todos os utilizadores. Não é uma limitação do miControl nem do Tauri; é o modelo de confiança do Windows. Qualquer promessa em contrário é marketing, self-signed disfarçado, ou crime.