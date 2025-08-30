# Description du projet

Le dossier courant est un fork de kZero, un projet permettant d'entrainer des AI sur des jeux de plateau. Je cherche à ajouter le jeu de trictrac à la liste des jeux déjà supportés (chess, go, ataxx).

Je développe le jeu de trictrac dans le dossier ../../trictrac/. Les règles du jeu et l'état d'une partie sont implémentées dans 'store', l'application ligne de commande est implémentée dans 'client_cli', elle permet déjà de jouer contre un bot, ou de faire jouer deux bots l'un contre l'autre. Les stratégies de bots sont implémentées dans le dossier 'bot'.
Pour rendre l'application trictrac compatible avec kZero, j'ai créé TrictracBoard dans bot/src/trictrac_board.rs qui implémente le trait board_game::board::Board sur lequel se base kZero.

J'en suis maintenant à modifier mon fork de kZero. J'ai ajouté la dépendance à mon projet trictrac dans rust/Cargo.toml et fait de premières modifications dans rust/kz-tui/src/main.rs et rust/kz-util/src/game.rs.
Peux-tu m'aider à compléter rust/kz-core/src/mapping/trictrac.rs ? Comme pour go.rs ou chess.rs situés dans le même dossier, ce fichier doit implémenter les traits InputMapper, PolicyMapper et MuZeroMapper qui permettent de traduire l'état du jeu et les mouvements dans des tenseurs. Peut-être que ce que j'ai fait dans ../../trictrac/bot/src/training_common.rs et ../../trictrac/bot/src/burnrl/environment.rs peut aider, il y a des fonctions de traduction de mouvement trictrac en index (TrictracAction::to_action_index) et de l'état du jeu en vecteur (TrictracState::from_game_state) que j'avais développées pour une autre solution d'entrainement d'AI (avec burnrl).

Résumé de la session pour l'intégration de Trictrac dans kZero

Objectif : Intégrer le jeu de Trictrac dans le framework kZero en implémentant les traits de mapping
nécessaires.

Fichier principal : rust/kz-core/src/mapping/trictrac.rs

Déroulement :

1.  Analyse : Nous avons analysé les implémentations existantes pour les échecs (chess.rs) et le Go (go.rs)
    dans kZero afin de comprendre l'architecture attendue. Nous avons également étudié votre code existant
    dans le projet trictrac (training_common.rs et game.rs) pour comprendre la logique du jeu.

2.  Planification : Après une discussion et une correction de ma compréhension des règles du Trictrac
    (notamment grâce à la structure GameState et la fonction to_string_id), nous avons convenu d'un plan
    pour l'implémentation des traits :
    - `PolicyMapper`: Utiliser votre énumération TrictracAction et TRICTRAC_ACTION_SPACE (taille 51) pour
      mapper les mouvements vers et depuis un index entier.
    - `InputMapper`: Représenter l'état du jeu du point de vue du joueur actif (POV) en utilisant :
      - Des plans binaires (`bools`) pour la position des dames sur les talons, l'état des dames (sur le
        plateau ou sorties) et l'étape du tour (TurnStage).
      - Des valeurs scalaires (`scalars`) pour les dés, et les statistiques de chaque joueur (points,
        trous, can_bredouille, can_big_bredouille).
    - `MuZeroMapper`: Mettre en place une structure de base pour l'encodage des mouvements.

3.  Réalisation : J'ai écrit et sauvegardé une première version complète de
    rust/kz-core/src/mapping/trictrac.rs contenant la structure TrictracStdMapper et l'implémentation des
    trois traits mentionnés, conformément au plan.

Prochaine étape identifiée :
Intégrer le nouveau module trictrac.rs dans le reste de la caisse kz-core, en commençant par le déclarer
dans rust/kz-core/src/mapping/mod.rs.
