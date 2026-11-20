# Profile: Konten auf einem Rechner, offline

Ein Rechner kann samudo selbst betreiben, auf dem Loopback, ein Dienst je
Konto. Jeder ist ein **Profil** in `~/.config/samu/config.toml`:

- **mit `upstream`** ist das Profil eine **Replik** des eigenen Servers:
  Es hält einen Spiegel und einen Ausgang, sodass Web-Oberfläche,
  Kommandozeile und Emacs weiterarbeiten, während der Server nicht
  erreichbar ist, und aufholen, wenn er wieder da ist;
- **ohne `upstream`** ist es ein **lokaler Server**: Das Konto und seine
  Daten verlassen den Rechner nie (zum Beispiel ein Arbeitskonto, das auf
  dem Arbeitslaptop bleiben muss).

```toml
default = "privat"

[profile.privat]                   # spiegelt den Server
account = "privat"
upstream = "https://samu.example.org"
port = 8490

[profile.arbeit]                   # nur auf diesem Rechner
account = "arbeit"
port = 8491

[front]                            # wahlweise: http://<profil>.samu.localhost
port = 80
```

Kommandozeile, Emacs und Desktop-Widget nehmen das Profil `default` oder
das mit `--profile` / `SAMU_PROFILE` genannte.
Ein Profil bringt sein Konto mit: `--account` / `SAMU_ACCOUNT` mit einem
anderen Konto wird abgelehnt (Exit 2), statt aus dem Konto des Profils zu
antworten. Dann das andere Profil wählen oder den Server mit `--server`
nennen.

## Einrichten

```sh
make install                                      # samu
make install-replica                              # samu-server und die User-Unit samu@.service
samu-server profile arbeit account add arbeit     # ein lokales Profil braucht sein Konto
systemctl --user enable --now samu@privat samu@arbeit
samu profile list                                 # läuft? angemeldet?
samu --profile arbeit login
```

Jedes Profil läuft als `samu-server profile <name>` auf `127.0.0.1` mit
seinem Port, die Daten in `~/.local/share/samu/<name>`; Logs mit
`journalctl --user -u samu@<name>`.

Ein neues `samu-server`-Binary braucht
`systemctl --user restart samu@<name>`: ein weiterlaufendes Profil behält das
alte Binary und damit das alte Schema, denn Migrationen laufen beim Start des
Servers. Neue Routen antworten dann mit 404, und der Export bleibt bei
`schema N, this build at M` stehen.

## Bei einer Replik anmelden

Wie gewohnt anmelden. Die Replik reicht Konto und Passwort an den Server
weiter; gelingt das, behält sie die Sitzung des Servers für sich und gibt
dem Client eine eigene, die auch offline funktioniert. Die erste Anmeldung
auf einem neuen Rechner braucht eine Verbindung. Was man über die Replik
schreibt, trägt auf dem Server das Gerät der Replik.

Agenten melden sich nicht bei einer Replik an; sie sprechen direkt mit dem
Server (`--server https://samu.example.org`).

## Im Browser

Jedes Profil hat seinen eigenen Hostnamen,
`http://<profil>.samu.localhost:<port>`, und damit sein eigenes Cookie:
Mehrere Konten sind nebeneinander angemeldet. Browser lösen `*.localhost`
von selbst auf den Loopback auf.

Ohne Ports, über die **Front**: ein kleiner Systemdienst auf Port 80, der
jede Verbindung an das Profil gibt, das ihr Hostname nennt.

```sh
make install-front               # braucht sudo: /etc/systemd/system/samu-front.service
```

Dann `http://privat.samu.localhost`, `http://arbeit.samu.localhost`, und
`http://samu.localhost` listet die Profile. Ein neues Profil wird ohne
Neustart erkannt; ein neues Binary `samu-server` braucht
`sudo systemctl restart samu-front`.

## Kalender eines lokalen Profils

Ein lokales Profil ruft seine Kalender selbst ab und braucht einen eigenen
geheimen Schlüssel in `~/.config/samu/<profil>.env`, den `samu@.service`
liest:

```sh
(umask 077; echo "SAMU_SECRET_KEY=$(openssl rand -base64 32)" > ~/.config/samu/arbeit.env)
systemctl --user restart samu@arbeit
```

Diesen Schlüssel nie ersetzen: Ein neuer kann die gespeicherten Quellen
nicht öffnen. Eine Replik ruft keine Kalender ab; das ist Sache des
Servers.

## Ein lokales Profil sichern

Ein lokales Profil hat keinen Server hinter sich, also wird es auf dem
Rechner gesichert: `samu-export@.timer` exportiert es alle 15 Minuten in
ein git-Repository ohne Remote unter
`~/.local/share/samu/<profil>/export` (Anhänge daneben, nicht in git).

```sh
systemctl --user enable --now samu-export@arbeit.timer
git -C ~/.local/share/samu/arbeit/export log --oneline | head
```
