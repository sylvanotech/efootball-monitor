<p align="center">
  <img src="assets/logo.svg" alt="efootball-monitor logo" width="140">
</p>

<h1 align="center">efootball-monitor</h1>

**Never trust the bars.**

Marche sous **Windows, macOS et Linux**. Gratuit et open source.

*[English version](README.md)*

Le jeu affiche 5 barres, et pourtant tes passes partent avec une demi-seconde de retard. Cet outil regarde le trafic
réseau d'eFootball pour te montrer ce qui se passe vraiment : sur quel serveur tu joues (ou contre quel adversaire, en
peer-to-peer), la latence et la perte de paquets, pendant tout le match.

Et surtout, tu vois tout ça **avant d'accepter le match**. Trop loin, trop de ping ? Tu refuses et tu choisis le
suivant.

## Pourquoi

Les barres donnent une vague idée, une seule fois, quand le match est trouvé. Elles ne disent pas où est le serveur,
si tu es en peer-to-peer, ni ce qui se passe après le coup d'envoi. Avec le monitor, tu as tout ça :

| | Barres du jeu | efootball-monitor |
|---|---|---|
| Localisation serveur / adversaire | non | fournisseur cloud + région, ou pays/ville |
| Client-server ou peer-to-peer | non | oui |
| Savoir avant d'accepter un match | non | localisation + latence sur l'écran d'acceptation |
| Latence en ms | non | au début du match, puis chaque seconde |
| Perte de paquets | non | oui, avec un message quand elle bouge |
| Pendant le match | non | oui, jusqu'à la fin |
| Historique | non | `matchlogs.csv` |

## Accepter ou refuser ?

Quand le jeu te trouve un adversaire, tu as quelques secondes pour accepter ou refuser. C'est là que le monitor bipe
et t'affiche où part le match et la latence. Serveur à Tokyo à 180 ms alors que tu es à Paris ? Refuse dans le jeu et
relance une recherche.

Avec `--overlay` (Windows), une petite fenêtre reste au-dessus du jeu avec juste l'essentiel :

```text
CLIENT-SERVER  ·  Google Cloud asia-northeast1
Server: Tokyo, Japan
182 ms    0.0% loss
```

L'outil n'accepte et ne refuse rien à ta place, c'est toi qui décides dans le jeu.

## Ce que ça fait

- Détecte chaque match et indique s'il est en client-server (via un relais Konami) ou en peer-to-peer.
- Affiche où est le serveur (ex. `Tokyo, Japan`, `Johannesburg, South Africa`, d'après sa région Google Cloud ou
  AWS), ou le pays et la ville de l'adversaire si tu ajoutes la base IP2Location.
- Mesure le temps de réponse du serveur au début du match à partir de l'échange de connexion du jeu, puis la latence
  chaque seconde sous forme de petit graphique, avec un avis GOOD / OK / FAR pour t'aider à accepter ou refuser.
- Calcule la perte de paquets (serveur/adversaire vers toi) et affiche une ligne quand elle varie de plus de 4 points.
- Bip quand un match est trouvé, la fenêtre peut rester réduite.
- Décode le test de vitesse que le jeu lance avant le matchmaking : latence et pertes vers chaque serveur Konami
  testé (commande `speed`).
- Enregistre chaque match dans `matchlogs.csv` (durée, serveur, latence, pire perte). Au bout de quelques soirées, tu
  vois quels serveurs et quels horaires te réussissent.
- Petite fenêtre toujours au premier plan avec le match en cours (`--overlay`, Windows), ou un fichier texte pour un
  overlay OBS.
- Peut enregistrer une session en `.pcap`, la rejouer, et l'anonymiser avant de la partager.

L'outil lit seulement une copie du trafic, comme Wireshark. Il ne bloque, ne retarde et ne modifie aucun paquet, et ne
touche ni aux fichiers ni à la mémoire du jeu.

## Windows, macOS, Linux

Pas seulement Windows : le même outil tourne nativement sur les trois, avec les mêmes fonctions. Utilise la machine
que tu as.

| | Windows | macOS | Linux |
|---|---|---|---|
| Jouer sur PC | ✓ | (eFootball n'existe que sous Windows) | ✓ avec Proton / Steam Deck |
| Jouer sur PlayStation / Xbox | ✓ | ✓ | ✓ |
| Fenêtre overlay (`--overlay`) | ✓ | | |

Joueurs console : lance-le sur n'importe quel ordinateur branché à un routeur ou un switch qui sait recopier le port
de la console (voir [Prérequis](#prérequis)).

Les versions macOS et Linux sont récentes : si tu les essaies, tes retours sont les bienvenus dans les issues.

## Exemple

```text
── Match 3 - 21:04:12 ──────────────────────────────────────
  Client-server (you and your opponent both play through this server)
  Server:   Tokyo, Japan (Google Cloud asia-northeast1)
  Ping:     182 ms   FAR - consider declining

── Match 4 - 21:05:40 ──────────────────────────────────────
  Client-server (you and your opponent both play through this server)
  Server:   Paris, France (AWS eu-west-3)
  Ping:     38 ms   GOOD
  21:05  ping avg 41 ms (38-52), loss 0.0%
  21:06  ping avg 44 ms (38-97), loss 0.0%
  ! 21:07:12 ping spike 212 ms
  ! 21:07:55 packet loss 12.5% (was 0.0%)
  21:07  ping avg 52 ms (39-212), loss 12.5%
  ...
  ping ▁▁▂▁▁▁▃▁▁▂▁▁▁▅▁▁▁▂▁▁ 41 ms  min 38 avg 42 max 97  loss 0.0%  09:28
  Match over - 9 min 28 s - ping avg 41 ms (min 38, max 97) - worst loss 12.5%
```

(Valeurs inventées. Dans le terminal, le ping et les barres sont colorés : vert, jaune, orange, rouge.)

## Démarrage rapide (Windows)

1. Installe [Npcap](https://npcap.com/#download) avec les options par défaut.
2. Récupère `efootball-monitor-windows-x64.zip` dans [Releases](../../releases) et extrais-le.
3. Lance `efootball-monitor.exe`, choisis ta carte réseau, lance eFootball.

Pour la fenêtre au premier plan, lance-le avec `efootball-monitor.exe --overlay` (ou ajoute `--overlay` à la fin de la
cible d'un raccourci). Le jeu doit être en mode fenêtré ou fenêtré sans bordure : Windows n'affiche rien par-dessus
le plein écran exclusif.

Pour savoir où sont les adversaires en peer-to-peer, ajoute les fichiers gratuits IP2Location (voir
[Installation](#installation)).

Projet communautaire, sans lien avec Konami.

## Prérequis

- **Windows 10/11** : [Npcap](https://npcap.com/#download) (gratuit, du projet Nmap), options par défaut.
- **Linux** : `libpcap`, en général déjà installé. Lancer avec `sudo`, ou une fois pour toutes :
  `sudo setcap cap_net_raw,cap_net_admin+eip ./efootball-monitor`
- **macOS** : rien à installer. Lancer avec `sudo`.

Sur PC, lance-le sur la machine où tourne le jeu. L'Ethernet c'est mieux, mais le Wi-Fi marche en général.

Sur PlayStation / Xbox, le PC doit voir le trafic de la console. Il faut un routeur ou un switch avec port mirroring
(recopie du port de la console vers celui du PC). Les consoles du réseau local sont reconnues automatiquement.

Les VPN marchent s'ils créent une carte réseau normale ou « RAW IP » (c'est le cas de WireGuard). En gros : si
Wireshark voit le trafic du jeu sur cette carte, l'outil aussi.

## Installation

1. Windows uniquement : installe [Npcap](https://npcap.com/#download).
2. Télécharge l'archive pour ton système dans [Releases](../../releases) et extrais-la quelque part.
3. Optionnel : la base gratuite IP2Location LITE DB3, pour localiser les adversaires en peer-to-peer.
   - Crée un compte gratuit sur <https://lite.ip2location.com>.
   - Télécharge DB3 LITE BIN pour l'IPv4 (`IP2LOCATION-LITE-DB3.BIN`), et l'IPv6 si tu veux
     (`IP2LOCATION-LITE-DB3.IPV6.BIN`).
   - Mets les fichiers `.BIN` à côté du programme.

   Sans ces fichiers, les serveurs Konami affichent quand même leur ville et leur pays (`Tokyo, Japan`,
   `Paris, France`...), mais les adversaires en peer-to-peer apparaissent en `unknown`.

## Utilisation

Lance le programme, choisis ta carte réseau, puis joue normalement. Laisse la fenêtre ouverte (réduite, ça va).

```text
efootball-monitor              # demande quelle carte utiliser
efootball-monitor 2            # utilise la carte 2
efootball-monitor --list       # affiche juste la liste des cartes
```

Pendant que ça tourne, tu peux taper :

- `speed` + Entrée : détail du test de vitesse (latence min/moy/max et pertes par serveur Konami)
- `servers` + Entrée : serveurs de test trouvés jusqu'ici, avec leurs adresses
- `help` + Entrée : commandes et couleurs

Options :

| Option | Défaut | |
|---|---|---|
| `[NUMÉRO]` | demandé | carte réseau |
| `--geo-dir <DOSSIER>` | dossier du programme | où sont les fichiers `.BIN` IP2Location |
| `--match-log <FICHIER>` | `matchlogs.csv` | historique des matchs |
| `--speed-servers <FICHIER>` | `speedtest.txt` | serveurs du test de vitesse (mis à jour tout seul) |
| `--overlay` | désactivé | petite fenêtre toujours au premier plan avec le match en cours (Windows). Glisse-la pour la déplacer, clic droit pour la cacher jusqu'au prochain match |
| `--overlay-file <FICHIER>` | désactivé | écrit l'état en direct dans un fichier texte (source texte OBS etc.) |
| `--record <FICHIER>` | désactivé | enregistre aussi le trafic du jeu dans un `.pcap` |
| `--max-ping <MS>` | `120` | les matchs à ce ping ou plus sont marqués `FAR` |
| `--show-ip` | désactivé | affiche les adresses IP et les ports (masqués par défaut) |
| `--ascii` | désactivé | caractères simples, si le graphique s'affiche en carrés dans une vieille console |

## Lire l'affichage

**Quand un match est trouvé**, un bloc s'affiche avec le numéro et l'heure du match, le mode, un lieu, et le ping
mesuré par le jeu lui-même au début du match. Attention à quel lieu c'est :

- **Client-server** affiche `Server:`, le serveur Konami auquel toi et ton adversaire êtes connectés. Dans ce mode, on
  ne peut pas voir où est ton adversaire.
- **Peer-to-peer** affiche `Opponent:`, l'endroit où est ton adversaire (il faut les fichiers IP2Location, sinon
  `unknown`).

Le ping est accompagné d'un avis :

- `GOOD` : moins de 60 ms
- `OK` : entre 60 ms et la limite `--max-ping` (120 ms par défaut)
- `FAR - consider declining` : à la limite ou au-dessus, mieux vaut refuser

**Pendant le match**, une ligne se met à jour chaque seconde : un petit graphique des 30 dernières secondes (une
barre par seconde, plus haute = plus lent), le ping actuel, le min / la moyenne / le max exacts sur ces 30 secondes,
la perte de paquets et le temps de jeu. Couleurs : vert sous 50 ms, jaune sous 100 ms, orange sous 200 ms, rouge
au-dessus ou pas de réponse.

Au-dessus, une ligne reste dans l'historique chaque minute avec les chiffres exacts de cette minute
(`21:06  ping avg 44 ms (38-97), loss 0.0%`) : tu peux remonter et revoir tout le match. Chaque fois que le ping
dépasse `--max-ping`, `! ping spike 212 ms` s'affiche avec l'heure.

En client-server, le ping en direct vient de nos propres sondes vers le serveur (connexion TCP, ou ping pour les
serveurs sur le port 5735), donc il peut être un peu différent de la mesure du jeu affichée au début. En peer-to-peer,
il vient des messages keep-alive que le jeu échange avec l'adversaire.

**`! packet loss x% (was y%)`** apparaît quand la perte varie de plus de 4 points. Le jeu envoie ses données par
groupes de 256 messages, donc la perte est comptée par groupe, dans le sens serveur/adversaire → toi.

**Fin de match** : après 8 s sans trafic (client-server) ou 3 s (peer-to-peer), un résumé d'une ligne s'affiche. Un
match qui se termine en moins de 20 s a sûrement été refusé ou annulé. En client-server, ça peut aussi vouloir dire
que le jeu est passé en TCP, que l'outil ne voit pas.

**Test de vitesse** : avant le matchmaking, le jeu ping plein de serveurs Konami. Quelques secondes après, un tableau
montre le ping moyen vers chacun. `speed` donne le tableau complet.

Les adresses IP sont masquées par défaut (pratique en stream). Utilise `--show-ip` pour les voir.

### matchlogs.csv

Une ligne par match de plus de 90 s :

`MatchTime, IP, Port, Region, MatchMode, Provider, MatchDuration, InitialDelay(ms), MinDelay(ms), AvgDelay(ms), MaxDelay(ms), MaxLossRate`

## Enregistrer et partager une session

```text
efootball-monitor --record session.pcap                    # joue normalement, le trafic est enregistré
efootball-monitor replay session.pcap                       # rejoue la session, instantanément
efootball-monitor replay session.pcap --realtime            # rejoue à la vitesse d'origine
efootball-monitor replay session.pcap --realtime --overlay  # pareil, avec la fenêtre overlay
efootball-monitor anonymize session.pcap session-anon.pcap  # nettoie avant de partager
```

Les captures Wireshark (`.pcap`, `.pcapng`) marchent aussi. `replay` n'a pas besoin de Npcap, ne contacte aucun
serveur (pas de mesure de latence) et ne touche ni à `matchlogs.csv` ni à `speedtest.txt`.

Les captures aident vraiment : une mise à jour du jeu peut changer les messages réseau, et seul du vrai trafic permet
de vérifier que la détection marche encore. Lance `anonymize` avant. Ça remplace ton IP, celles des autres joueurs et
les adresses MAC, et ça vire tout ce qui n'est pas du trafic de jeu (DNS etc.). Les adresses des serveurs Konami sont
gardées. Les paquets du jeu eux-mêmes ne sont pas modifiés, donc ne partage que ce que tu veux bien partager.

Pour signaler un problème, ouvre une issue avec le modèle *Detection problem* et joins le fichier anonymisé.
[fixtures/README.md](fixtures/README.md) (en anglais) explique comment en faire un test de non-régression.

## Dépannage

**`Npcap is not installed`** : installe-le depuis <https://npcap.com/#download> et relance le programme.

**`No network interfaces found`** : sous Windows, réinstalle Npcap. Si tu as coché « Restrict Npcap driver's access to
Administrators only », lance en administrateur. Sous Linux/macOS, utilise `sudo`.

**Rien ne s'affiche quand un match est trouvé** : sûrement la mauvaise carte réseau, essaie un autre numéro. Ça peut
aussi être un match en TCP, ou le port mirroring pas configuré (consoles).

**La latence est toujours `X`** : le serveur ne répond pas aux sondes depuis ton réseau (pare-feu, box). Les serveurs
sur le port 5735 utilisent le ping, qui demande les droits admin sous Windows. Les matchs et la perte de paquets sont
quand même détectés.

**Les localisations affichent `unknown`** : mets les fichiers `.BIN` IP2Location à côté du programme, ou utilise
`--geo-dir`.

**Mauvais mode, match raté, chiffres bizarres** : enregistre avec `--record`, anonymise, ouvre une issue.

## Compiler

Il faut [Rust](https://rustup.rs) 1.84+.

```sh
cargo build --release
cargo test               # tests unitaires + replay de chaque capture de fixtures/
```

Sous Linux, installe `libpcap-dev` avant. Sous Windows, récupère le [Npcap SDK](https://npcap.com/#download) puis
`set LIBPCAP_LIBDIR=C:\chemin\vers\npcap-sdk\Lib\x64`.

Comment les matchs sont détectés : [docs/PROTOCOL.md](docs/PROTOCOL.md) (en anglais).
Mettre à jour les listes d'IP cloud : `python3 tools/update_cloud_ranges.py`.

## Crédits

- Basé sur l'idée de l'outil *eFootball Network Monitor* de la chaîne eFootballCheaterReport. Ce projet en est une
  implémentation open source séparée.
- Peut utiliser les données IP2Location LITE de <https://lite.ip2location.com> (CC BY-SA 4.0). Non incluses.
- Plages d'IP cloud publiées par [Google Cloud](https://www.gstatic.com/ipranges/cloud.json) et
  [AWS](https://ip-ranges.amazonaws.com/ip-ranges.json).
- eFootball est une marque de Konami Digital Entertainment.

Licence : [MIT](LICENSE-MIT) ou [Apache-2.0](LICENSE-APACHE), au choix.
