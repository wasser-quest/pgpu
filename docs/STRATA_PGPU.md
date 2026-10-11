# pgpu mit Strata: Einrichtung und Verwendung

Diese Anleitung verbindet Router, GPU-Instanz, Strata und Praxis. Details:

- [Modelle, Quantisierung, RAM, Vast-Suchqueries und Preise](strata/PGPU_MODELS.md)
- [OrcaRouter und Hugging-Face-Zugang](strata/ORCAROUTER_DOCKER.md)
- [Aufbau und Build des Strata-pgpu-Images](strata/IMAGE.md)
- [RunPod-Backend und Einschränkungen](runpod.md)
- [RunPod-Strata-Beispielkonfiguration](../deploy/config.runpod-strata.toml)

Die Strata-Seiten sind Kopien aus dem
[Strata-Fork](https://github.com/wasser-quest/dockerstrata-image). Engine-Code,
Modelldownload und Monitoring-Scripts liegen dort, nicht im pgpu-Repository.

## 1. Was läuft wo?

| Komponente | Ort | Aufgabe |
| --- | --- | --- |
| pgpu-Router | NAS/VPS/PC | GPU mieten, Lifecycle, Budgets, Dashboard und API-Proxy |
| NetBird auf dem Router | gleicher Netzwerk-Namespace oder Host | privater Zugang zum Router und den GPU-Peers |
| gpu-agent | GPU-Container | Status, Befehle, Terminal und Health |
| Strata | GPU-Container | Modell herunterladen, vorbereiten und Antworten erzeugen |
| Praxis oder anderer Client | eigener Rechner/Container | OpenAI-kompatible API verwenden |

Ports in den Beispielen:
- Router-Dashboard/API: `8080`.
- Router-Passthrough zur Strata-API: `11434`; Weboberfläche: `11435`.
- gpu-agent auf der GPU: `9100`, normalerweise Loopback mit NetBird-Forwarding.
- Strata auf der GPU: `8080`. Das ist nicht derselbe Rechner wie das Dashboard.

Das GPU-Image ist **kein zweiter Router**. RunPod-Unterstützung wird im
Router-Image gebaut; Orca/HF/Quantisierung im Strata-Image.

## 2. Image- und Feature-Stand prüfen

Ein laufender Container mit Tag `latest` wird durch einen neuen Push nicht
aktualisiert. Für reproduzierbare Deployments einen geprüften Digest verwenden.
Vor Updates Versions-/Commitstand sowie das Registry-Manifest prüfen.

```bash
docker compose pull
docker compose up -d --force-recreate router
```

Für die neuen Features muss das passende Image gebaut sein:
- `provider="runpod"`: RunPod-fähiger pgpu-Router, initiales Backend.
- `MODEL_VARIANT`/`QUANTIZATION`: Strata-Image mit dem neuen Entrypoint.
- `HF_TOKEN`: Strata-Image mit `tools/hf_auth.py`.
- Huge-Page-Workaround als Standard: neuer Strata-pgpu-Build.

HF-Authentifizierung und Huge-Page-Workaround benötigen einen aktuellen Build
aus dem Strata-Fork; alte Images bekommen diese Funktionen nicht durch neue
Env-Werte. Bei Bedarf beide Strata-Buildstufen neu bauen und veröffentlichen.
Das initiale RunPod-Backend hat automatisierte Tests, aber noch keinen bezahlten
Live-Test. Ein Git-Commit allein bestätigt keine Registry-Veröffentlichung.

## 3. Secrets unterscheiden

| Variable | Empfänger | Bedeutung |
| --- | --- | --- |
| `ROUTER_TOKEN` | Router und dessen Bediener | Dashboard-/Management-Anmeldung |
| `VAST_API_KEY` | Router | Vast-Ressourcen verwalten |
| `RUNPOD_API_KEY` | Router | RunPod-Ressourcen verwalten |
| `NB_API_TOKEN` | Router | NetBird-Setup-Keys erzeugen |
| `NB_SETUP_KEY` | NetBird-Peer | Peer anmelden; GPU-Key wird vom Router erzeugt |
| `PRAXIS_NODE_TOKEN` | GPU-Agent | pro Instanz vom Router erzeugt, **nicht** Dashboard-Token |
| `API_KEY` | Strata und Client | Strata-Inference autorisieren |
| `HF_TOKEN` | GPU-Container | Modellrepository lesen, wenn Auth nötig ist |

Private `.env` und private Routerkonfiguration nicht committen. Echte Tokens
nicht in Chats oder Screenshots posten. Bereits veröffentlichte Tokens
widerrufen und ersetzen. `HF_TOKEN_FILE` ist eine Alternative, wenn eine
Secret-Datei in den GPU-Container gemountet werden kann.

## 4. Minimaler Router-Stack

Voraussetzung: Docker Compose, NetBird-Zugang, `/dev/net/tun`, passendes
Strata-GPU-Image und ein gültiger Anbieter-Key. Nur **einen** automatischen
Router für dieselben verwalteten Ressourcen betreiben.

In einem eigenen Deployment-Verzeichnis eine private `config.toml`, `.env` und
`compose.yml` anlegen. Beispiel-Compose (keine öffentlichen Port-Mappings):

```yaml
services:
  netbird:
    image: netbirdio/netbird:latest # nach Prüfung einen Digest pinnen
    cap_add: [NET_ADMIN]
    devices: ["/dev/net/tun:/dev/net/tun"]
    environment:
      NB_MANAGEMENT_URL: ${NB_MANAGEMENT_URL:?setzen}
      NB_SETUP_KEY: ${NB_SETUP_KEY:?setzen}
      NB_HOSTNAME: pgpu-strata
    volumes:
      - netbird-config:/etc/netbird
    restart: unless-stopped

  router:
    image: ${PGPU_IMAGE:-vayayo/praxis-gpu-router:latest}
    network_mode: "service:netbird"
    command: ["--config", "/config/config.toml"]
    volumes:
      - router-data:/data
      - ./config.toml:/config/config.toml:ro
    environment:
      ROUTER_TOKEN: ${ROUTER_TOKEN:?setzen}
      VAST_API_KEY: ${VAST_API_KEY:-}
      RUNPOD_API_KEY: ${RUNPOD_API_KEY:-}
      NB_API_TOKEN: ${NB_API_TOKEN:?setzen}
      RUST_LOG: info
    depends_on: [netbird]
    restart: unless-stopped

volumes:
  router-data:
  netbird-config:
```

Private `.env` mit Platzhaltern (lokal ersetzen):

```dotenv
NB_MANAGEMENT_URL=https://YOUR-NETBIRD-MANAGEMENT
NB_SETUP_KEY=<router-peer enrollment key>
NB_API_TOKEN=<NetBird API token>
ROUTER_TOKEN=<mindestens 32 zulässige Zeichen>
VAST_API_KEY=<Vast key oder leer bei RunPod>
RUNPOD_API_KEY=<RunPod key oder leer bei Vast>
PGPU_IMAGE=vayayo/praxis-gpu-router:latest
```

Die Datenpfade in der TOML müssen zum Mount passen: hier
`router.data_dir="/data"`. Für RunPod ein **frisches Deployment/Volume** verwenden,
z.B. einen anderen Compose-Projektnamen. Niemals die Vast-Datenbank übernehmen.
Das RunPod-Beispiel hat `/data-runpod`: für dieses Compose auf `/data` ändern
oder einen passenden zusätzlichen persistenten Mount bereitstellen.

Start und Prüfung:

```bash
chmod 600 .env
# Router image normally runs as UID/GID 10001; it must read the bind-mounted config.
sudo chown 10001:10001 config.toml
sudo chmod 600 config.toml
docker compose up -d netbird
docker compose exec netbird netbird status
docker compose up -d router
docker compose logs -f router
```

Das Compose enthält keinen STT-Sidecar. Chat funktioniert ohne ihn; Spracherkennung
benötigt zusätzlich den passenden STT-Dienst, siehe die vorhandenen Deployment-Beispiele.

## 5. NetBird und Routerkonfiguration

Der Router braucht Zugang zu derselben NetBird-Management-Instanz wie die GPUs.
In der privaten TOML setzen:

```toml
[router]
bind_ip = "0.0.0.0" # nur im privaten Namespace, kein öffentliches Publishing
dashboard_port = 8080
data_dir = "/data"

[netbird]
api_url = "https://YOUR-NETBIRD-MANAGEMENT"
management_url = "https://YOUR-NETBIRD-MANAGEMENT"
groups = ["YOUR-GPU-GROUP"]
router_nb_ip = "auto"
router_nb_port = 8080
```

Die Gruppen müssen existieren. Einrichtung im Router/NetBird-Dashboard prüfen.
Gruppenzuweisung allein erlaubt keinen Verkehr: mindestens Router → GPU
TCP `9100` und `8080`, GPU → Router TCP `8080` sowie Client → Router
TCP `8080`, `11434`, `11435` erlauben. Die Policy gilt im richtigen Overlay,
nicht in einer anderen NetBird-Management-Instanz.

Dashboard: `http://<Router-NetBird-IP-oder-Name>:8080`. Ohne API-Key niemals
Strata außerhalb Loopback bereitstellen. Client-Zugriff nur über das gewünschte
Overlay/ACL und authentifizierte API; keine öffentlichen Proxyports öffnen.

## 6. Anbieter und Modelle auswählen

### Vast

Aus der [Strata-Vast-Konfiguration](https://github.com/wasser-quest/dockerstrata-image/blob/main/vastai/pgpu-config.toml)
eine private Kopie erstellen. GPU-Image setzen:

```toml
[[slots]]
id = 1
role = "llm"
name = "Strata"
image = "vayayo/strata-pgpu:latest"
mode = "on_demand"
disk_gb = 150
search_query = 'num_gpus=1 gpu_ram>=24 cpu_ram>=64 cpu_cores_effective>=24 disk_space>=150 inet_down>=500 inet_down_cost<=0.01 disk_bw>=1500 cuda_vers>=13.0 compute_cap>=7.5 reliability2>=0.95'
passthrough = { 11434 = "api", 11435 = "webui" }

[slots.services]
api = { port = 8080, health = "/health", busy = "inflight" }
webui = { port = 8080, health = "/", busy = "inflight" }
agent = { port = 9100, busy = "none" }
```

Dieses Fragment ersetzt nur den betreffenden Slot, nicht Budgets, Limits und
NetBird-Einstellungen. Keine doppelten TOML-Tabellen erzeugen. Bei Orca lieber
96 GB Host-RAM, 32 GB+ VRAM und 180 GB Disk suchen; die tatsächlich zugewiesenen
Ressourcen danach prüfen. Suchquerys garantieren weder Container-RAM noch AVX-512.

### RunPod

[Beispiel](../deploy/config.runpod-strata.toml) kopieren und
[RunPod-Anleitung](runpod.md) beachten. `provider="runpod"` steht auf oberster
TOML-Ebene. `search_query=""`; Ressourcen über `[runpod]` wählen. Das GPU-Image
auf das tatsächlich veröffentlichte Strata-Image setzen. Der Beispiel-GHCR-Tag
ist keine Zusage, dass dort ein Image existiert.

RunPod legt Strata-Daten in `/workspace/strata` auf dem Pod-Volume ab. Stoppen
behält das Volume, Terminieren löscht es. Reale Kosten inklusive Storage mit
RunPod abgleichen; ein Compute-Cap ist keine Garantie für die Gesamtrechnung.

### Normal Qwen

```toml
[slots.env]
MODEL_VARIANT = "standard"
FAMILY = "qwen"
QUANTIZATION = "IQ2_XS"
CONTEXT = "32768"
VISION = "no"
LOW_RAM = "auto"
HOST = "0.0.0.0"
STRATA_NO_LARGEPAGES = "1"
```

Andere Originalgrößen: `Q2_0`, `IQ3_XXS`, `IQ3_S`. Coder und Unsloth sind
separate Familien; die [Modelltabelle](strata/PGPU_MODELS.md) enthält die gültigen
Kombinationen und RAM-Empfehlungen.

### OrcaRouter mit privatem Hugging-Face-Token

```toml
[slots.env]
MODEL_VARIANT = "orcarouter"
QUANTIZATION = "IQ3_XXS"
CONTEXT = "32768"
VISION = "no"
HOST = "0.0.0.0"
HF_TOKEN = "<private repository read token>"
STRATA_NO_LARGEPAGES = "1"
```

Nur `IQ3_XXS` ist im Orca-Containerworkflow freigegeben; **nicht `IQ3_XSS`**.
Token mit Leserecht verwenden und gegebenenfalls Repository-Bedingungen vorher
akzeptieren. `HF_TOKEN` nur am Router zu setzen genügt nicht: hier gehört es in
`[slots.env]`, damit es der GPU beim Erstellen mitgegeben wird. Der neue
HF-fähige Strata-Build ist Voraussetzung.

pgpu injiziert Node-/NetBird-Tokens automatisch. Diese niemals mit leeren
Beispielwerten überschreiben. Die Strata-API-Key-Generierung kann im Image einen
privaten Key erstellen; für stabile Clients einen eigenen Key privat verwalten.

## 7. Modell wechseln und Start beobachten

`[slots.env]` wird bei **Instanzerstellung** übergeben. Eine TOML-Änderung oder
ein Router-Neustart verändert nicht die Umgebung einer bestehenden GPU-Instanz.
Dort muss die tatsächliche Containerumgebung aktualisiert und der Container
neu gestartet werden. Ein bloßer Supervisor-Neustart mit alten Env-Werten reicht
für solche Änderungen nicht. Nicht unüberlegt löschen: Downloads/Volumes gehen
je nach Anbieter verloren. Bereits heruntergeladene Größen können bei erhaltener
Speicherung wiederverwendet werden.

Startphasen: Image-Pull → Pakete/Engineprüfung → Modellshards → Pack-Konvertierung
→ MTP-Download → MTP-Quantisierung → Engine-/RAM-/GPU-Cache-Laden → healthy.
Für Packen und Cache-Laden gibt es nicht immer einen zuverlässigen Prozentwert.

Monitoring-Scripts liegen im Strata-Fork unter `tools/`:

```bash
python3 tools/watch_pgpu_progress.py --url http://YOUR-ROUTER:8080
python3 tools/watch_pgpu.py --url http://YOUR-ROUTER:8080 --once
```

Token wird verdeckt abgefragt. Ersteres zeigt Balken für Standard/Orca und Setup-
Phasen. Beide zugehörigen Dateien (`watch_pgpu_progress.py` und
`pgpu_progress_probe.py`) aus demselben aktuellen Checkout verwenden. Diese
Scripts müssen aus einem Commit stammen, der sie enthält. Direkt im
GPU-Container funktioniert das Shell-Script:

```bash
bash tools/watch_strata_local.sh
```

Das muss dort vorhanden sein; `/root/watch-strata.sh` war eine manuell installierte
Kopie auf einzelnen Instanzen und ist nicht automatisch in jeder Instanz verfügbar.

### Wake / Startzustände

Im aktualisierten Router schickt ein Slot-`wake` vorhandenen gestoppten,
verwalteten GPUs direkt einen Startauftrag, bis das Warm-Pool-Ziel erreicht ist.
Ohne vorhandene GPU plant der Reconciler weiterhin eine neue Miete. Budget-,
Pin-/Lock- und Pending-Operation-Prüfungen bleiben aktiv. Wiederholtes Wake
sendet bei einem bereits laufenden Startauftrag keinen doppelten Start.
Ein bestätigtes `start_failed` kann durch erneutes explizites Wake wiederholt
werden; ein unbestätigter Auftrag wird nicht blind erneut gesendet.

- `start_requested`: Start beauftragt, Zuweisung noch nicht bestätigt.
- `scheduling`: Anbieter meldet Warten auf GPU-Kapazität (oder bestätigt nach
  dem Startauftrag weiterhin eine gestoppte Allocation).
- `provisioning` / `booting`: Instanz wird eingerichtet beziehungsweise Engine startet.
- `healthy`: Backend ist tatsächlich bereit.

Provider-/Admission-Fehler beim direkten Wake-Start erscheinen als HTTP-Fehler
und `wake_failed`-Event, nicht bloß als „waking“. `force=true` kann die
Auto-Miete wieder einschalten, hebt aber Budgets und Locks nicht auf. Die
Wake-Änderung erfordert einen Router-Build ab Version 0.26.5 und Container-Neustart.

### Zu lange im Scheduling: nichts tun, stoppen oder löschen

Optional pro Slot, **vor** anderen TOML-Untertabellen wie `[slots.env]` oder
als eigene Untertabelle des betreffenden `[[slots]]` ergänzen:

```toml
[slots.scheduling]
after_s = 1800       # 30 Minuten seit dem aktuellen Startauftrag
action = "stop"     # "none" | "stop" | "destroy"
```

- `none`: keine Timeout-Aktion; Standard. `after_s=0` deaktiviert sie ebenfalls.
- `stop`: gestoppte Allocation anfordern, Daten behalten. Storage-Kosten können
  weiterlaufen — es ist kein kostenloser Zustand.
- `destroy`: Instanz terminieren; lokale Daten beziehungsweise das RunPod-Pod-
  Volume können endgültig verloren gehen. Nur bewusst aktivieren.

Der Timeout gilt nur für `start_requested` und `scheduling`, nicht für Modell-
Download/Boot in `provisioning`, `booting`, `agent_connected` oder `healthy`.
Gemessen wird seit dem Startauftrag, nicht seit Eintritt in `scheduling`.
Die Aktion folgt beim nächsten erfolgreichen Inventory-/Reconcile-Tick nach
Ablauf; ohne frischen Providerstatus wird nicht aufgrund alter Daten gelöscht.
Ein aktuelles `running` verhindert die Scheduling-Cleanup-Aktion ebenfalls.

Bei `stop`/`destroy` wird **der ganze Slot** persistent auf Hold gesetzt,
damit kein sofortiger Neustart oder Ersatz entsteht. Das gilt auch bei
Warm-Zeitfenstern und geplanten Wakes; andere Instanzen dieses Slots werden
von der normalen Sleep-Policy behandelt. Ein **expliziter API-/Dashboard-Wake**
hebt den Hold wieder auf. Noch ausstehende Stop-/Delete-Aufträge müssen zuerst
abgeschlossen oder sicher storniert werden. Andere Slots bleiben unverändert.

Pins, Slot-Locks und manuell verwaltete Instanzen werden nicht automatisch
bereinigt. Providerfehler bleiben als persistenter, erneut versuchter Auftrag
sichtbar; erfolgreiches Stoppen/Löschen wird nicht vorgetäuscht. Vor Wiederholung
wird geprüft, ob die Allocation oder Regel inzwischen gewechselt hat; dann
wird der alte Cleanup-Auftrag storniert. Der Hold bleibt bis zum expliziten Wake.

Diese Einstellung benötigt den entsprechend aktualisierten Router-Build.

## 8. Praxis und andere Clients verbinden

Praxis braucht den OpenAI-kompatiblen **llamacpp**-Adapter, nicht Ollama:

```dotenv
USE_PROVIDER=llamacpp
LLAMACPP_API_BASE=http://YOUR-ROUTER:11434
LLAMACPP_MODEL=qwen3.8-flash-next-iq2_xs
LLAMACPP_API_KEY=<Strata key>
```

Bei Orca lautet die Modell-ID
`orcarouter-qwen3.8-flash-next-uncensored-iq3_xxs`. Andere IDs über `/v1/models`
ermitteln. Praxis hängt `/v1` selbst an; andere OpenAI-Clients erwarten häufig
`http://YOUR-ROUTER:11434/v1` als Base URL. Nach Config-/Secret-Änderungen Praxis
neu starten und bestehende Chat-Overrides kontrollieren.

Der Key ist **nicht** der Router-Dashboard-Token. Automatisch erzeugte Keys stehen
auf der GPU in `$STRATA_DATA/api-key.txt`: bei Vast normalerweise `/data`, im
RunPod-Beispiel `/workspace/strata`. Nur privat ansehen, nicht posten.

## 9. Häufige Fehler

| Problem | Prüfen |
| --- | --- |
| HTTP 404 mit `ollama` | Falscher Provider/API-Pfad; `llamacpp` verwenden |
| HTTP 401 mit `llamacpp` | Tatsächlich geladener Strata-Key, neue Instanz, Secret-Store und Neustart |
| Terminal „Session beendet“ | Agentverbindung/TermOpen; Connector-Debuglogs statt nur UI-Meldung |
| NetBird „invalid auto groups“ | Existenz und erwartetes Namens-/ID-Format der GPU-Gruppe |
| Strata FATAL bei Orca | Schreibweise `IQ3_XXS`, unterstützte Quantisierung, HF-Zugriff |
| `expert-profile.bin` fehlt | Altes Image; neues Image oder Strata-Script `copy_pgpu_profile.py` |
| Downloadanzeige 0 Bytes | Agenttracking kennt eventuell Pfad/Total nicht; Logs und Balkenmonitor prüfen |
| „still starting“ minutenlang | Engine-Log, CPU/RAM-Limits, Memory Pressure; THP-Workaround testen |
| Wenig tok/s | CPU-Quota/AVX2, Cache-Treffer, MTP-Akzeptanz und Timingprofil; Idle-GPUwerte sind kein Decode-Messwert |

Routerdiagnose:

```yaml
# environment des router-Service
RUST_LOG: "info,praxis_router::connector=debug"
```

```bash
docker compose up -d --force-recreate router
docker compose logs -f router
```

GPU-Diagnose:

```bash
supervisorctl -c /etc/supervisor/conf.d/strata-pgpu.conf status
tail -n 30 /workspace/logs/strata.log
tail -n 30 /opt/strata/strata-iq2_xs.log
cat /sys/fs/cgroup/cpu.max
cat /sys/fs/cgroup/memory.max
cat /proc/pressure/memory
nvidia-smi
```

Orca-Engine-Log: `/opt/strata/strata-orcarouter-iq3_xxs.log`. Logausgaben können
Secrets enthalten: vor Weitergabe prüfen/schwärzen. Nicht alle Fehler sind durch
mehr VRAM lösbar; auf dem wirklichen Host messen. Keine Garantie für 150 tok/s.

## 10. Sicher aktualisieren

1. Geprüfte Git-Commits und Image-Digests notieren.
2. Routerkonfiguration und Router-/NetBird-Identitätsdaten privat sichern.
3. Images bauen/pullen; eine Änderung im Git-Repository allein reicht nicht.
4. Router rekonstruieren; GPU-Image-Updates kontrolliert mit erhaltener Speicherung
   durchführen. RunPod-Ephemeral-Root und Volume unterscheiden.
5. Health, Modell-ID und Client-Key prüfen; einen kurzen Request testen.
6. Anbieterrechnung und gestoppte Storage-Ressourcen kontrollieren.

`docker compose down -v` löscht Volumes: nicht als Standard-Update verwenden.
Manuell gelöschte GPU-Instanzen können als Warmup-Fail in der Router-Blacklist
landen; das allein beweist keinen defekten Host.
