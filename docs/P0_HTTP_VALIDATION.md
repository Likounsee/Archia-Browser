# P0-HTTP-04 — validation du transport HTTP/1.1

## Périmètre

Cette tâche porte sur la validation stricte du framing HTTP/1.1 et sur le chemin réel de transport. Elle ne certifie pas le navigateur complet et ne ferme pas P0-FUZZ-14.

## Invariants vérifiés dans le code et les tests

- Requête : limite du corps avant DNS/connexion, plafond du nombre et de la taille des en-têtes, validation des noms/valeurs, rejet de l'injection CRLF, des doublons de casse, des corps GET/HEAD et des framing ambigus.
- Réponse : rejet des versions/status malformés, des en-têtes dupliqués (sauf Set-Cookie), de Content-Length invalide, de Transfer-Encoding non pris en charge ou en conflit avec Content-Length, et des corps interdits/tronqués.
- Chunking : tailles bornées, calculs d'offset contrôlés, extensions/trailers validés, rejet des trailers qui changent les métadonnées de représentation et des octets après le terminateur.
- Ressources : plafonds de corps/en-têtes et rejet avant d'ajouter un bloc de lecture si la taille projetée dépasse la limite.
- Intégration : les tests http_transport_round_trips_a_response_over_a_real_loopback_socket et http_transport_rejects_ambiguous_and_truncated_framing_over_loopback exercent HttpTransport::send via une vraie socket locale, pas seulement le parseur isolé.

## Conditions de clôture

1. cargo fmt --all -- --check, tests workspace, build et Clippy réussissent sur le SHA candidat.
2. Les jobs Linux, Windows et macOS de la CI ainsi que les audits de dépendances sont terminés sans échec.
3. La campagne de fuzzing HTTP a terminé et les crashs/reproducteurs éventuels ont été triés ; P0-FUZZ-14 reste une tâche distincte.
4. La roadmap mentionne explicitement le SHA testé et les éventuels risques résiduels.

Un test écrit ou une exécution sur un ancien SHA n'est pas une preuve pour le candidat courant.
