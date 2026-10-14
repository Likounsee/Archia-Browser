# Prompt de continuation — Archia Browser

Tu travailles sur `Likounsee/Archia-Browser`, exclusivement sur la branche `Archia-Browser`.

## Objectif
Faire progresser rapidement le moteur de navigateur Rust en ajoutant de vraies fonctionnalités testées, en corrigeant les régressions et en maintenant une documentation cohérente avec le code réel.

## Règles impératives
1. Inspecte le code et les tests existants avant de modifier quoi que ce soit. Ne te fie pas uniquement aux README ou à une estimation précédente.
2. Fais des changements concrets et ciblés, idéalement une fonctionnalité cohérente par lot. Évite les refactorisations massives sans bénéfice mesurable.
3. Ajoute ou adapte des tests de régression pour chaque comportement nouveau. Couvre les cas limites et les entrées malformées lorsque c'est pertinent.
4. Vérifie le formatage Rust (`cargo fmt --all -- --check`), les tests, la compilation et Clippy via la CI du SHA exact. Une CI en attente ou un simple passage du formatage ne vaut pas une validation complète. Si la CI échoue, lis les logs et corrige avant d'empiler d'autres changements.
5. Après toute modification fonctionnelle, mets à jour toutes les documentations **concernées** : `README.md`, `ROADMAP_P0_P3.md`, `docs/ARCHITECTURE.md`, ainsi que le modèle de menace/sécurité ou les documents P0 lorsque leurs garanties ou leur statut changent. Ne modifie pas un rapport de validation explicitement historique pour lui faire prétendre qu'il décrit le HEAD courant.
6. N'invente ni fonctionnalités, ni résultats de tests, ni état CI. Distingue clairement « implémenté », « testé localement », « CI verte » et « conforme au standard ».
7. Utilise l'API GitHub pour lire la version la plus récente des fichiers et son SHA avant chaque mise à jour. Toutes les écritures doivent cibler `branch: "Archia-Browser"`. Ne réutilise pas un SHA périmé.
8. N'intègre aucun moteur complet existant (Chromium/Blink, Gecko, WebKit ou Servo). Garde les couches HTML, DOM, CSS, layout, rendu, réseau, sécurité et plateforme séparées.
9. Les pourcentages sont des estimations de couverture fonctionnelle, pas des taux de tests ni des scores de compatibilité. Ne les augmente que lorsqu'une fonctionnalité significative a réellement été livrée et explique brièvement pourquoi.

## Format de compte rendu à chaque réponse
- En français, va vite mais reste explicatif : indique les changements réalisés, pourquoi ils sont utiles et les limites restantes.
- Donne toujours les pourcentages actuels pour :
  - Tokenizer HTML
  - Parser HTML
  - DOM
- Mentionne les commits et liens CI pertinents.
- Signale explicitement si la CI est en cours ou en échec ; ne dis jamais qu'elle est verte sans preuve sur le dernier SHA.
- Termine par la prochaine action concrète, sans demander à l'utilisateur de choisir lorsque la suite logique est évidente.

## État récent à reprendre
Le DOM a reçu des API de recherche par attribut, valeur d'attribut, classe, attribut `name`, premier résultat et combinaisons tag+attribut/tag+classe. Les tests couvrent les correspondances exactes et l'ordre du document. Une exécution CI précédente a signalé uniquement des différences de formatage Rust ; une correction a été poussée ensuite. Vérifie impérativement l'état CI du HEAD avant de déclarer ce lot validé.

## Priorité technique immédiate
1. Vérifier/corriger la CI après les changements DOM et documentation.
2. Continuer les améliorations standards HTML/DOM qui apportent un gain clair, avec tests de régression.
3. Garder la roadmap et les pourcentages honnêtes, en privilégiant les comportements réellement implémentés plutôt que des promesses générales.
