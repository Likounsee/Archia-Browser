# Archia Browser — Roadmap P0 à P3

Branche de référence : `Archia-Browser`.

Ce fichier suit les travaux restants connus. Un item ne sera terminé qu'après implémentation, tests de régression exécutés et validation CI. P0 reste ouvert ; P1–P3 sont planifiés et ne sont pas déclarés réalisés.

## P0 — Sécurité et corrections bloquantes

- [ ] Auditer URL/origines de bout en bout entre parser, policy, loader, cache, cookies, redirects et transport local.
- [ ] Exécuter et compléter les tests HTTP framing, headers, trailers, limites, EOF et redirects.
- [ ] Revalider la frontière réseau/`file://`, les chemins locaux, les redirects et la suppression des credentials inter-origines.
- [ ] Finaliser l'audit cookies : PSL, Domain/Path, Secure/HttpOnly, préfixes, Max-Age/Expires, SameSite et suppression.
- [ ] Finaliser l'audit cache : clés, Vary, credentials, no-store/private, réponses personnalisées et requêtes bloquées.
- [ ] Vérifier limites HTML/CSS, parsing malformé, complexité des sélecteurs, expansion `var()` et fuzzing adversarial.
- [ ] Appliquer réellement le budget mémoire aux allocations critiques ou définir explicitement son rôle comme simple métrique.
- [ ] Auditer panics, indexations, overflows, allocations et nettoyage après erreurs/annulation.
- [ ] Corriger toute régression de compilation, formatage ou tests révélée par CI.
- [ ] Exécuter `cargo fmt --all -- --check`, `cargo test --workspace` et `cargo build --workspace` sur CI et conserver les liens de résultats.

## P1 — Fiabiliser les fondations

- [ ] Améliorer la construction/récupération HTML, entités, parcours DOM, mutations et namespaces.
- [ ] Compléter parser CSS, cascade complète, valeurs calculées, at-rules, pseudo-éléments et tests de conformité.
- [ ] Corriger le formatage inline, inline-block, dimensions intrinsèques, overflow, viewport et ordre de peinture.
- [ ] Définir un système de fontes, charger des fontes locales, gérer fallbacks et améliorer shaping/Unicode.
- [ ] Fiabiliser cycle de vie Page/Document/Tab, annulation des navigations et réponses tardives.
- [ ] Structurer les erreurs, diagnostics sans secrets, documentation d'invariants, benchmarks et fuzzing.
- [ ] Ajouter corpus de référence HTML/CSS/layout et tests de non-régression automatisés.

## P2 — Fonctionnalités web essentielles

- [ ] Implémenter Flexbox, layout de tableaux, scrolling, stacking contexts et `z-index`.
- [ ] Ajouter images comme éléments remplacés, `object-fit`, border-radius, opacity et transforms de base.
- [ ] Implémenter HTTPS/TLS natif avec validation stricte de certificats et nom d'hôte.
- [ ] Compléter cache HTTP, cookies, stockage persistant, téléchargements et annulation réseau.
- [ ] Améliorer formulaires, focus, clavier, événements et accessibilité de base.
- [ ] Construire UI fenêtre/onglets, barre d'adresse, navigation, historique, favoris, paramètres et gestionnaire de téléchargements.
- [ ] Compléter les abstractions plateforme et valider Windows, Linux et ArchiaOS dans la mesure possible.
- [ ] Ajouter tests d'intégration du parcours navigation → réseau → HTML/CSS → layout → rendu.

## P3 — Runtime, isolation et capacités avancées

- [ ] Choisir et intégrer/implémenter un runtime JavaScript sans intégrer un moteur de navigateur complet.
- [ ] Ajouter DOM bindings, événements, timers, Fetch, tâches/microtasks et APIs web essentielles.
- [ ] Imposer quotas CPU/mémoire, interruption des scripts et nettoyage lors des navigations.
- [ ] Définir et implémenter isolation de processus/site et sandbox OS avec permissions minimales.
- [ ] Compléter politiques Same-Origin, CORS, CSP, mixed content et permissions avant d'exposer les scripts.
- [ ] Ajouter console, inspecteur DOM/style, panneau réseau et profilage CPU/mémoire.
- [ ] Profiler puis optimiser layout/rendu incrémental ; évaluer accélération GPU avec fallback logiciel.
- [ ] Ajouter profils, mode privé, gestion des permissions par site, packaging signé et mises à jour sûres.
- [ ] Mettre en place matrice de compatibilité, audits de sécurité et critères de publication mesurables.

## Règles de validation

1. Reproduire chaque bug avec un test avant correction lorsque possible.
2. Vérifier les limites de temps, mémoire, taille et les chemins d'erreur.
3. Exécuter formatage, tests workspace et build ; ne pas assimiler des tests ajoutés à des tests passés.
4. Corriger les échecs CI avant d'empiler d'autres changements.
5. Enregistrer commits, résultats CI et risques résiduels ici.
6. Si une nouvelle faille P0 apparaît, la corriger avant de poursuivre les phases suivantes.

## État à la création

- P0 reste ouvert : plusieurs correctifs et limites ont été publiés, mais les vérifications des derniers changements ne sont pas confirmées.
- P1–P3 représentent le backlog restant connu, à réviser au fil des audits ; ce n'est pas une garantie d'exhaustivité.
- Toutes les modifications de cette roadmap sont destinées à la branche `Archia-Browser`, jamais à `main` et sans création de branche supplémentaire.
