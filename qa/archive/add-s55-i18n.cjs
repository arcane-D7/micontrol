const fs = require('fs');
const path = require('path');

const dir = path.join(__dirname, '..', 'src', 'i18n');

const translations = {
  en: {
    debloaterTitle: 'Windows Debloater',
    debloaterDesc: 'Catalog of system-cleaning actions to reduce Windows resource usage: telemetry, pre-installed apps, unused services and background processes. Each option is a real uninstall/apply with a matching reinstall/restore path — nothing here is a blind toggle.',
    debloaterInspiration: 'Inspired by tiny11builder (NTDEV), Win11Debloat (Raphire) and Sophia Script (farag2). You decide how soft or aggressive the optimization gets — we only provide the tools.',
    debloaterOpen: 'Open Debloater',
    debloatDone: 'removed / applied',
    debloatRestored: 'reinstalled / restored',
    debloatUninstall: 'Uninstall',
    debloatReinstall: 'Reinstall',
    debloatUninstallTitle: 'Remove / apply this optimization',
    debloatReinstallTitle: 'Reinstall / restore to Windows default',
    tierGreenTitle: 'Green — no risk',
    tierGreenDesc: 'Safe and fully reversible. Reinstall any removed app from the Microsoft Store.',
    tierYellowTitle: 'Yellow — may break non-essential features',
    tierYellowDesc: 'May affect Xbox gaming features, OneDrive sync or optional services. These are not essential to Windows and can be reinstalled.',
    tierYellowConsent: 'I understand these may break non-essential functionality — enable actions',
    tierRedTitle: 'Red — may destabilize Windows',
    tierRedDesc: 'Aggressive changes touching integral parts of the system. Read the warning before enabling.',
    tierRedAck: 'I am aware that these actions may cause Windows instability and, in case of a system failure, data loss may occur. MiControl is NOT responsible for any damage or loss caused. I want to enable these actions.',
  },
  pt: {
    debloaterTitle: 'Windows Debloater',
    debloaterDesc: 'Catálogo de ações de limpeza do sistema para reduzir o consumo de recursos do Windows: telemetria, apps pré-instalados, serviços não usados e processos em segundo plano. Cada opção é um uninstall/apply real com caminho de reinstall/restore correspondente — nada aqui é um toggle cego.',
    debloaterInspiration: 'Inspirado no tiny11builder (NTDEV), Win11Debloat (Raphire) e Sophia Script (farag2). O nível de otimização, suave ou agressiva, é sempre responsabilidade do usuário — nós só damos as ferramentas.',
    debloaterOpen: 'Abrir Debloater',
    debloatDone: 'removido / aplicado',
    debloatRestored: 'reinstalado / restaurado',
    debloatUninstall: 'Desinstalar',
    debloatReinstall: 'Reinstalar',
    debloatUninstallTitle: 'Remover / aplicar esta otimização',
    debloatReinstallTitle: 'Reinstalar / restaurar ao padrão do Windows',
    tierGreenTitle: 'Verde — sem risco',
    tierGreenDesc: 'Seguras e totalmente reversíveis. Reinstale qualquer app removido pela Microsoft Store.',
    tierYellowTitle: 'Amarelo — podem quebrar funções não essenciais',
    tierYellowDesc: 'Podem afetar recursos de jogos Xbox, sincronização do OneDrive ou serviços opcionais. Não são essenciais ao Windows e podem ser reinstalados.',
    tierYellowConsent: 'Entendo que estas ações podem quebrar funções não essenciais — habilitar botões',
    tierRedTitle: 'Vermelho — podem desestabilizar o Windows',
    tierRedDesc: 'Mudanças agressivas que tocam partes integrais do sistema. Leia o aviso antes de habilitar.',
    tierRedAck: 'Estou ciente de que estas ações podem causar instabilidades no Windows e, caso haja alguma falha de sistema, poderão haver perdas de dados. O miControl NÃO se responsabiliza por nenhum dano ou perda causada. Quero habilitar estas ações.',
  },
  es: {
    debloaterTitle: 'Windows Debloater',
    debloaterDesc: 'Catálogo de acciones de limpieza del sistema para reducir el consumo de recursos de Windows: telemetría, apps preinstaladas, servicios sin usar y procesos en segundo plano. Cada opción es un uninstall/apply real con su ruta de reinstall/restore correspondiente — nada aquí es un toggle ciego.',
    debloaterInspiration: 'Inspirado en tiny11builder (NTDEV), Win11Debloat (Raphire) y Sophia Script (farag2). El nivel de optimización, suave o agresivo, es siempre responsabilidad del usuario — nosotros solo damos las herramientas.',
    debloaterOpen: 'Abrir Debloater',
    debloatDone: 'eliminado / aplicado',
    debloatRestored: 'reinstalado / restaurado',
    debloatUninstall: 'Desinstalar',
    debloatReinstall: 'Reinstalar',
    debloatUninstallTitle: 'Eliminar / aplicar esta optimización',
    debloatReinstallTitle: 'Reinstalar / restaurar al patrón de Windows',
    tierGreenTitle: 'Verde — sin riesgo',
    tierGreenDesc: 'Seguras y totalmente reversibles. Reinstala cualquier app eliminada desde la Microsoft Store.',
    tierYellowTitle: 'Amarillo — pueden romper funciones no esenciales',
    tierYellowDesc: 'Pueden afectar funciones de juegos Xbox, sincronización de OneDrive o servicios opcionales. No son esenciales para Windows y pueden reinstalarse.',
    tierYellowConsent: 'Entiendo que estas acciones pueden romper funciones no esenciales — habilitar botones',
    tierRedTitle: 'Rojo — pueden desestabilizar Windows',
    tierRedDesc: 'Cambios agresivos que tocan partes integrales del sistema. Lee la advertencia antes de habilitar.',
    tierRedAck: 'Soy consciente de que estas acciones pueden causar inestabilidad en Windows y, en caso de fallo del sistema, podría haber pérdida de datos. miControl NO se responsabiliza de ningún daño o pérdida causada. Quiero habilitar estas acciones.',
  },
  fr: {
    debloaterTitle: 'Windows Debloater',
    debloaterDesc: "Catalogue d'actions de nettoyage système pour réduire la consommation de ressources de Windows : télémétrie, apps préinstallées, services inutilisés et processus en arrière-plan. Chaque option est un véritable uninstall/apply avec son chemin de reinstall/restore correspondant — rien ici n'est un toggle aveugle.",
    debloaterInspiration: 'Inspiré de tiny11builder (NTDEV), Win11Debloat (Raphire) et Sophia Script (farag2). Le niveau d\'optimisation, doux ou agressif, reste toujours la responsabilité de l\'utilisateur — nous ne fournissons que les outils.',
    debloaterOpen: 'Ouvrir le Debloater',
    debloatDone: 'supprimé / appliqué',
    debloatRestored: 'réinstallé / restauré',
    debloatUninstall: 'Désinstaller',
    debloatReinstall: 'Réinstaller',
    debloatUninstallTitle: 'Supprimer / appliquer cette optimisation',
    debloatReinstallTitle: 'Réinstaller / restaurer aux valeurs par défaut de Windows',
    tierGreenTitle: 'Vert — sans risque',
    tierGreenDesc: 'Sûres et entièrement réversibles. Réinstallez toute app supprimée depuis le Microsoft Store.',
    tierYellowTitle: 'Jaune — peuvent casser des fonctions non essentielles',
    tierYellowDesc: 'Peuvent affecter les fonctions de jeu Xbox, la synchronisation OneDrive ou des services optionnels. Non essentielles à Windows et réinstallables.',
    tierYellowConsent: "Je comprends que ces actions peuvent casser des fonctions non essentielles — activer les boutons",
    tierRedTitle: 'Rouge — peuvent déstabiliser Windows',
    tierRedDesc: "Modifications agressives touchant des parties intégrales du système. Lisez l'avertissement avant d'activer.",
    tierRedAck: "Je suis conscient que ces actions peuvent causer des instabilités dans Windows et, en cas de défaillance du système, des pertes de données peuvent se produire. miControl n'est PAS responsable de tout dommage ou perte causée. Je souhaite activer ces actions.",
  },
};

// New red-tier item translations (title/desc/why per locale)
const itemTranslations = {
  en: {
    red_copilot: { title: 'Disable & remove Copilot', desc: 'What it is: the Microsoft Copilot AI assistant app and its system integration. What this does: applies a policy disabling Copilot and uninstalls the app.', why: 'Why: removes a background AI service most users never use, freeing RAM and CPU cycles.' },
    red_edge_preinstall: { title: 'Block Edge pre-launch & background', desc: 'What it is: Edge keeps pre-launching and running background processes even when closed. What this does: applies Edge policies disabling Startup Boost, background mode and pre-launch.', why: 'Why: Edge no longer silently consumes RAM/CPU when you are not using it. Edge itself is NOT uninstalled.' },
    red_widgets: { title: 'Disable Widgets (News & Interests)', desc: 'What it is: the Windows Widgets feed with news/weather on the taskbar. What this does: disables the Widgets service via policy.', why: 'Why: removes a persistent background data-fetching process and taskbar resource usage.' },
    red_cortana_relics: { title: 'Remove Cortana', desc: 'What it is: the legacy Cortana voice assistant. What this does: disables the Cortana policy and uninstalls the app.', why: 'Why: eliminates an unused voice-assistant background app. Reinstallable from the Store.' },
  },
  pt: {
    red_copilot: { title: 'Desabilitar e remover o Copilot', desc: 'O que é: o app assistente de IA Microsoft Copilot e sua integração com o sistema. O que faz: aplica política que desabilita o Copilot e desinstala o app.', why: 'Por quê: remove um serviço de IA em segundo plano que a maioria nunca usa, liberando RAM e ciclos de CPU.' },
    red_edge_preinstall: { title: 'Bloquear pré-inicialização e segundo plano do Edge', desc: 'O que é: o Edge mantém processos pré-inicializados em segundo plano mesmo fechado. O que faz: aplica políticas do Edge desabilitando Startup Boost, modo em segundo plano e pre-launch.', why: 'Por quê: o Edge para de consumir RAM/CPU silenciosamente quando você não o usa. O Edge NÃO é desinstalado.' },
    red_widgets: { title: 'Desabilitar Widgets (News & Interests)', desc: 'O que é: o feed de Widgets do Windows com notícias/clima na barra de tarefas. O que faz: desabilita o serviço de Widgets via política.', why: 'Por quê: remove um processo persistente de busca de dados em segundo plano e o consumo na barra de tarefas.' },
    red_cortana_relics: { title: 'Remover a Cortana', desc: 'O que é: a assistente de voz Cortana legada. O que faz: desabilita a política da Cortana e desinstala o app.', why: 'Por quê: elimina um app de assistente de voz não usado em segundo plano. Reinstalável pela Store.' },
  },
  es: {
    red_copilot: { title: 'Desactivar y eliminar Copilot', desc: 'Qué es: la app de IA asistente Microsoft Copilot y su integración con el sistema. Qué hace: aplica una política que desactiva Copilot y desinstala la app.', why: 'Por qué: elimina un servicio de IA en segundo plano que la mayoría nunca usa, liberando RAM y ciclos de CPU.' },
    red_edge_preinstall: { title: 'Bloquear pre-lanzamiento y segundo plano de Edge', desc: 'Qué es: Edge mantiene procesos pre-lanzados en segundo plano incluso cerrado. Qué hace: aplica políticas de Edge desactivando Startup Boost, modo en segundo plano y pre-launch.', why: 'Por qué: Edge deja de consumir RAM/CPU silenciosamente cuando no lo usas. Edge NO se desinstala.' },
    red_widgets: { title: 'Desactivar Widgets (News & Interests)', desc: 'Qué es: el feed de Widgets de Windows con noticias/clima en la barra de tareas. Qué hace: desactiva el servicio de Widgets vía política.', why: 'Por qué: elimina un proceso persistente de captura de datos en segundo plano y el consumo en la barra de tareas.' },
    red_cortana_relics: { title: 'Eliminar Cortana', desc: 'Qué es: la asistente de voz Cortana heredada. Qué hace: desactiva la política de Cortana y desinstala la app.', why: 'Por qué: elimina una app de asistente de voz sin usar en segundo plano. Reinstalable desde la Store.' },
  },
  fr: {
    red_copilot: { title: 'Désactiver et supprimer Copilot', desc: "Qu'est-ce que c'est : l'app d'assistant IA Microsoft Copilot et son intégration système. Ce que ça fait : applique une politique désactivant Copilot et désinstalle l'app.", why: 'Pourquoi : supprime un service IA en arrière-plan que la plupart des utilisateurs ne utilisent jamais, libérant RAM et cycles CPU.' },
    red_edge_preinstall: { title: "Bloquer le pré-lancement et l'arrière-plan d'Edge", desc: "Qu'est-ce que c'est : Edge maintient des processus pré-lancés en arrière-plan même fermé. Ce que ça fait : applique des politiques Edge désactivant Startup Boost, le mode arrière-plan et le pré-lancement.", why: "Pourquoi : Edge cesse de consommer RAM/CPU silencieusement quand vous ne l'utilisez pas. Edge n'est PAS désinstallé." },
    red_widgets: { title: 'Désactiver les Widgets (News & Interests)', desc: "Qu'est-ce que c'est : le flux de Widgets Windows avec actualités/météo dans la barre des tâches. Ce que ça fait : désactive le service Widgets via politique.", why: 'Pourquoi : supprime un processus persistant de récupération de données en arrière-plan et la consommation dans la barre des tâches.' },
    red_cortana_relics: { title: 'Supprimer Cortana', desc: "Qu'est-ce que c'est : l'assistante vocale Cortana héritée. Ce que ça fait : désactive la politique Cortana et désinstalle l'app.", why: 'Pourquoi : élimine une app dassistante vocale inutilisée en arrière-plan. Réinstallable depuis le Store.' },
  },
};

for (const [locale, keys] of Object.entries(translations)) {
  const file = path.join(dir, `${locale}.json`);
  const data = JSON.parse(fs.readFileSync(file, 'utf8'));
  Object.assign(data.sysOpt, keys);
  // items
  data.sysOpt.items = data.sysOpt.items || {};
  const items = itemTranslations[locale];
  for (const [id, entry] of Object.entries(items)) {
    data.sysOpt.items[id] = entry;
  }
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + '\n');
  console.log(`${locale}: OK`);
}
