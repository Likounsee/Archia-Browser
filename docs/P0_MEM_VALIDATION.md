# P0-MEM-11 — comptabilité mémoire globale

## Ajout de cette tranche

Page::try_from_html_at_with_budget fournit un chemin de construction de page qui :
- réserve une estimation basée sur la taille HTML et CSS avant de construire les structures ;
- utilise des multiplications/additions vérifiées pour calculer la réservation initiale ;
- construit DOM, arbre stylé, layout et display list uniquement après réussite du préflight ;
- ajuste ensuite la réservation à l'estimation des structures retenues, avec échec fermé si la croissance est refusée ;
- conserve la réservation avec la page pour que Drop rende la charge au budget partagé.

## Tests ajoutés

- budget trop petit : échec sans fuite de comptabilité ;
- 1 000 cycles de création/destruction : retour du budget à zéro à chaque cycle ;
- clonage profond : charge séparée pour la copie, puis libération indépendante.

## Limites explicites

La mesure retenue est une estimation conservatrice et non une télémétrie exacte de l'allocateur. La construction peut échouer lors de l'ajustement après avoir créé temporairement des structures. Les allocations DOM/CSS/layout/réseau/cache ne sont pas toutes instrumentées individuellement, et les appels historiques à Page::from_html restent non budgétés pour compatibilité. P0-MEM-11 demeure donc partiel jusqu'à ce que les principaux chemins de production utilisent le budget partagé et que les autres critères de la roadmap soient validés par CI.


## Intégration du chemin de construction (tranche suivante)

Le chargeur de documents transfère maintenant sa réservation déjà active à `Page::try_from_html_at_with_reservation`. La même routine vérifie l'estimation structurelle, agrandit la réservation si nécessaire, refuse la page si le budget ne permet pas cet agrandissement et attache la réservation à la page. Cela évite de dupliquer la logique de réconciliation et permet de garder la réservation existante pendant le décodage HTML et la collecte des feuilles de style. Sans budget partagé configuré, le chemin historique demeure disponible pour compatibilité.


## Tranche renderer mémoire — allocations temporaires et display list

La display list réserve désormais sa capacité avec `Vec::try_reserve` avant de modifier ses compteurs ; `try_push` expose le résultat et le chemin historique `push` reste compatible. L'allocation de la liste d'indices enfants et la croissance de la pile de clips sont également fallibles : en cas d'échec, le rendu est tronqué/arrêté plutôt que de demander une allocation infallible supplémentaire.

Cette tranche réduit les chemins de panique OOM dans le rendu, mais ne constitue pas une comptabilité exacte au budget partagé : la liste d'indices, la pile de clips et les autres buffers temporaires ne sont pas tous imputés au `MemoryBudget`. La catégorie P0-MEM-11 reste ouverte jusqu'à l'instrumentation des principaux chemins d'allocation, la couverture de leurs erreurs/cancellations et la validation CI/fuzz du SHA candidat.
