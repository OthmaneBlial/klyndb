# Roadmap — terminer Klyndb

Document de relève, mis à jour le 3 octobre 2026. Le périmètre complet reste dans [PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md). Objectif : un client Rust + Tauri utilisable au quotidien pour remplacer DBeaver, gratuit, open source et sans compte.

## 1. Lire ceci avant de travailler

- Dépôt : https://github.com/OthmaneBlial/klyndb — travailler directement sur `main`.
- Site : https://othmaneblial.github.io/klyndb/.
- Faire des changements utiles et courts, mettre cette roadmap à jour, puis commit/push après chaque changement validé.
- GitHub Actions reste **désactivé**. Vérifications locales ciblées pour les changements courants ; suite complète pour les changements transversaux et les releases.
- Priorité : un produit qui fonctionne. Ne pas passer des jours sur les cas rares, les audits répétés ou la documentation historique.
- Aucun code commercial Beekeeper, aucun secret dans Git, aucun benchmark ou screenshot inventé.

## 2. Ce qui existe déjà

La base publiée est `b6e93fa` : neuf moteurs, SQL multi-onglets, résultats streamés/virtualisés, inspection et édition de tables, imports/exports, historique/requêtes sauvegardées, plans, diagrammes, TLS/identités client/SSH pour les moteurs documentés, interfaces MongoDB et Redis.

Les dernières fonctionnalités source ajoutent les raccourcis personnalisables, les confirmations SQL configurables, une optimisation de complétion et un correctif de cancellation SQL Server. La démo native de 84 secondes, le README et le site sont publiés.

**Attention :** le téléchargement `v0.1.0-preview.2` est plus ancien que ces changements source. C'est un aperçu macOS Apple Silicon, signé ad hoc et non notarizé. Ne pas annoncer une parité complète avec DBeaver ni une validation Windows/Linux.

Preuves et limites : [COMPATIBILITY.md](docs/COMPATIBILITY.md), [VALIDATION.md](docs/VALIDATION.md), [RELEASES.md](docs/RELEASES.md), [REFERENCE_MATRIX.md](docs/REFERENCE_MATRIX.md).

## 3. Première tâche de la relève : finir le catalogue PostgreSQL

Une implémentation **inachevée** est conservée dans [postgres-catalog.patch](docs/handoff/postgres-catalog.patch). Elle ajoute Types, Sequences, Users et Roles au navigateur existant : pages de 100, recherche, détails à la demande et ouverture du SQL d'inspection sans exécution.

```bash
git status --short
# Sur un checkout propre ; ne pas écraser du travail existant.
git apply docs/handoff/postgres-catalog.patch
```

Le patch contient les commandes Tauri, les types IPC, le driver PostgreSQL, le navigateur partagé et un test réel. Il ne contient pas de mots de passe. La copie locale est aussi sauvegardée dans le stash `cbdb1078769f19072b5924b8a8f7ba810999a207` ; appliquer le patch OU le stash, jamais les deux.

À reprendre dans cet ordre :

1. Libérer de l'espace de compilation. Le dernier build a échoué faute de place ; environ 2 Gio restaient après suppression du seul cache incrémental du projet. Ne pas effacer les données des fixtures, les releases ou les preuves.
2. Vérifier le correctif dans `crates/drivers/postgres/src/catalog.rs` : PostgreSQL réordonnait les filtres et appelait `has_sequence_privilege` sur une relation non séquence. Le garde `CASE WHEN relkind='S'` est ajouté mais **pas encore revalidé**.
3. Compiler, exécuter `postgres_catalog_paging_details_and_readonly_inspection`, puis vérifier le navigateur. Les détails de séquence ne doivent pas avancer sa valeur ; les rôles ne doivent jamais exposer de password/hash. Ouvrir un onglet ne doit pas exécuter la requête.
4. Vérifier aussi que Functions & procedures fonctionne toujours après le passage de `RoutineBrowser` à `CatalogBrowser`.
5. Mettre les guides à jour, commit/push sur `main`. Ne pas publier le patch comme fonctionnalité terminée avant ces checks.

Le build frontend et lint passaient avant les dernières retouches ; le nouveau test PostgreSQL a échoué sur le problème ci-dessus. Le build desktop suivant a été arrêté pour la relève. Aucun succès complet ni nouvelle release n'est revendiqué. Logs locaux : `artifacts/catalog-*.log`. Le runner local scellé `artifacts/run-catalog-focused.py` réutilise une fixture existante ; ne pas afficher ses secrets. Ces artifacts et le stash ne sont pas disponibles dans un clone distant. Le test échoué peut avoir laissé ses objets de fixture `klyndb catalog <uuid>` et `klyndb_{login,role}_<uuid>` : nettoyer seulement les objets identifiés comme appartenant à ce test.

## 4. Ensuite : terminer les usages quotidiens

- [ ] Explorateur : compléter databases, schemas, types, sequences, users/roles sur les moteurs qui les permettent ; chargement paresseux, recherche, refresh et permissions réelles.
- [ ] Éditeur : positions d'erreur pour davantage de moteurs, complétion/dialectes restants, raccourcis et comportements cohérents entre onglets.
- [ ] Connexions simultanées : isolation des onglets, transactions, changements en attente, cancellation, reconnect/disconnect et restauration sans rejouer de requêtes.
- [ ] Structure/DDL : terminer les définitions et modifications sûres de schéma, contraintes/triggers/statistiques et recréation ; expliciter les limites par moteur.
- [ ] Grille, plans et diagrammes : terminer les usages encore incomplets, gros schémas/résultats, disposition et export. Éviter toute perte de précision ou de données.
- [ ] Settings : compléter police/éditeur, formatage SQL, auto-commit, résultats et paramètres des drivers sans multiplier les écrans.
- [ ] SSH/TLS : compléter les moteurs/transports manquants, proxy/multi-hop/MFA et gestion des host keys ; garder vérification TLS et stockage natif des secrets.

## 5. Couverture des bases : réelle, pas seulement une URL acceptée

- [ ] Terminer les fonctionnalités manquantes et l'acceptance desktop des neuf moteurs actuels : PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, DuckDB, ClickHouse, MongoDB, Redis.
- [ ] Ajouter et vérifier CockroachDB, Redshift et TiDB. Réutiliser les protocoles existants quand possible, tout en gérant leurs différences.
- [ ] Étendre progressivement : Oracle, Cassandra, ScyllaDB, Firebird, LibSQL, BigQuery, Snowflake, DynamoDB, Trino, Presto, SurrealDB et SAP HANA lorsque praticable.
- [ ] XLSX/Parquet, backup/restore et interfaces spécialisées des moteurs supplémentaires.
- [ ] Mesurer le coût des drivers avant leur isolation/chargement optionnel ; extensions avec un modèle de sécurité après stabilisation du cœur.

Chaque ajout doit permettre un vrai workflow : connecter → parcourir → lire/interroger → modifier si supporté → exporter → déconnecter. Mettre la matrice de compatibilité à jour ; ne pas afficher des actions factices.

## 6. Performances et livraison

- [ ] Mesurer et conserver les résultats : démarrage froid/chaud et temps interactif, mémoire du processus complet au repos/1/5 connexions, 100k lignes, grands schémas et 100 onglets.
- [ ] Mesurer scrolling/frame rate, coût IPC, overhead des requêtes, débit du streaming et cancellation. Corriger les goulets mesurés sans inventer de comparaison DBeaver.
- [ ] Livrer et vérifier macOS Apple Silicon/Intel, Windows x64 et Linux x64. Ensuite envisager Windows/Linux ARM64.
- [ ] Produire les bons artifacts : macOS app/DMG, Windows installer/portable, Linux AppImage/deb et rpm si praticable.
- [ ] Signature/notarisation et updater signé lorsque les clés/infrastructures sont disponibles ; ne pas affaiblir la sécurité pour contourner leur absence.
- [ ] Nouvelle release incluant les changements source, après validation de ses **artifacts exacts** sur les plateformes visées.
- [ ] Garder README SVG/emojis, captures réelles, site et liens de téléchargement cohérents. Ajouter une vidéo native continue ; la démo actuelle est un montage de screenshots.

## 7. Vérifier sans ralentir chaque changement

```bash
# Frontend : choisir le fichier de test concerné.
npm --prefix apps/desktop run build
npm --prefix apps/desktop run lint
npm --prefix apps/desktop test -- src/routines.test.tsx

# Rust : formatting puis compiler/tester les cibles concernées.
cargo fmt --all -- --check

# Suite complète locale : changement transversal ou préparation de release.
./scripts/check.sh
```

Préserver la configuration des fixtures locales et les contrôles de sécurité. Éviter les changements inutiles de graphe Cargo : certains choix de cibles recompilent le C++ DuckDB et consomment beaucoup de disque. Une commande ignorée/skippée ou un build interrompu n'est pas un check réussi.

## 8. Quand considérer le travail terminé

Un développeur doit pouvoir installer une release, lancer rapidement, connecter plusieurs bases, parcourir le schéma, exécuter/annuler des requêtes, recevoir de gros résultats sans blocage, modifier avec revue/transactions, importer/exporter et fermer/rouvrir sans perdre son workspace ni rejouer des écritures.

- [ ] Ce workflow est accepté sur les plateformes et moteurs annoncés.
- [ ] Aucun contrôle présenté comme fonctionnel n'est factice ; limites et compatibilité sont documentées.
- [ ] Performances mesurées, dépendances/provenance auditées, secrets protégés et packages vérifiés.
- [ ] README, site, roadmap, guides et release décrivent la même version.
- [ ] Les exigences de PRODUCT_SPEC ont chacune une preuve adaptée à leur périmètre ; les extensions annoncées comme futures y restent explicitement identifiées. Ne pas réduire silencieusement la cible à la première preview.

Avancer dans cet ordre, livrer chaque tranche utile, et garder `main` buildable. L'historique détaillé reste dans Git et VALIDATION.md ; cette roadmap sert à agir.
