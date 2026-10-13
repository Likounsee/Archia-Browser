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


## Tranche renderer — suppression de buffers temporaires évitables

Le parsing des couleurs `rgb()/rgba()` et des rayons de bordures n'alloue plus de `Vec` temporaires pour trois/quatre composantes : il utilise des itérateurs et des tableaux de taille fixe. Les formes invalides et les listes de rayons trop longues restent rejetées. Un test vérifie les couleurs RGB/RGBA, les nombres de composantes invalides et l'expansion des rayons horizontaux/verticaux.

Avec la tranche précédente, cela couvre les buffers temporaires de parsing et les principaux chemins d'allocation explicites du rendu. Ce n'est toujours pas suffisant pour fermer P0-MEM-11 : les allocations internes DOM/CSS/layout/cache/réseau ne sont pas toutes instrumentées, et les chemins historiques non budgétés existent encore.


## Tranche DOM — croissance des enfants faillible

Le DOM expose maintenant `Node::try_append`, qui réserve la capacité avant de modifier le vecteur et rend le nœud à l'appelant si la réservation échoue. Le parseur HTML utilise ce chemin pour ajouter les nœuds et réserve aussi explicitement la capacité de sa pile d'éléments ouverts. En cas d'échec, il arrête le traitement des tokens restants plutôt que de poursuivre sur une allocation infallible. L'API historique `append` est conservée pour compatibilité.

Cette tranche ne rend pas toutes les allocations DOM faillibles : la normalisation de structure, les maps d'attributs, les piles de sous-arbres supprimés, le style tree et le layout nécessitent encore un traitement dédié. P0-MEM-11 reste donc ouvert.


## Tranche parseur — allocations auxiliaires faillibles

Le suivi des sous-arbres ignorés (limite de profondeur/nœuds) réserve explicitement la capacité de la pile et de l'index `HashMap`/vecteurs avant de les modifier. Si une réservation échoue, le parseur arrête le traitement des tokens restants. La fermeture des balises réserve sa pile temporaire avant de dépiler le moindre nœud : un échec laisse la pile originale intacte. `Node::insert_child` réserve aussi avant l'insertion.

Limite restante : la normalisation du document, les copies de chaînes/attributs et les allocations des arbres CSS/layout ne sont pas toutes contrôlées par un budget global ni entièrement fallibles. Cette tranche renforce la résistance à l'échec d'allocation, mais ne clôt pas P0-MEM-11.


## Tranche de récupération — fermeture de balises sans perte silencieuse

La fermeture d'une balise réserve désormais la capacité du parent pour tous les enfants détachés avant de modifier le sous-arbre. Si cette réservation échoue, les nœuds sont remis sur la pile d'éléments ouverts dans leur ordre d'origine. Si l'ajout au parent final échoue, le nœud est conservé sur cette pile pour la finalisation plutôt que d'être abandonné.

Cette mesure limite la perte de contenu lors d'une pression mémoire, mais ne transforme pas toutes les allocations de normalisation en opérations faillibles et ne remplace pas l'instrumentation globale du DOM/CSS/layout. P0-MEM-11 reste ouvert.
