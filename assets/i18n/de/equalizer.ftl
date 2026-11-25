equalizer-advanced = Erweitert
equalizer-simple = Einfach
equalizer-sound = Klang
equalizer-profile-default = Standard
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Eingefügtes Profil
equalizer-profile-name-hint = Profilname
equalizer-save-as = Als Profil speichern
equalizer-rename = Profil umbenennen
equalizer-delete = Profil löschen
equalizer-bass = Bass
equalizer-treble = Höhen
equalizer-tone-reset = Auf 0 dB zurücksetzen
equalizer-preamp = Vorverstärkung
equalizer-preamp-auto = Automatisch
equalizer-preamp-manual = Manuell
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = Okt.
equalizer-no-gain = —
equalizer-filters = Filter
equalizer-no-filters = Noch keine Filter. Füge einen hinzu, wähle oben ein Preset oder importiere ein Profil.
equalizer-add-filter = Filter hinzufügen
equalizer-remove-filter = Filter entfernen
equalizer-kind-peak = Peak
equalizer-kind-low-shelf = Low-Shelf
equalizer-kind-high-shelf = High-Shelf
equalizer-kind-low-pass = Tiefpass
equalizer-kind-high-pass = Hochpass
equalizer-kind-notch = Kerbfilter
equalizer-kind-band-pass = Bandpass
equalizer-kind-all-pass = Allpass
equalizer-transfer = Import und Export
equalizer-import = Importieren
equalizer-export = Exportieren
equalizer-import-file = Datei importieren…
equalizer-export-file = Datei exportieren…
equalizer-imported = { $name } importiert
equalizer-imported-skipped = { $name } importiert, ohne { $count ->
        [one] eine Zeile
       *[other] { $count } Zeilen
    }
equalizer-import-nothing = Keine Equalizer-Einstellungen zum Importieren gefunden
equalizer-import-too-many = Nicht importiert: { $count ->
        [one] ein Filter ist aktiv
       *[other] { $count } Filter sind aktiv
    }, mehr als ein Profil fasst
equalizer-import-too-large = Nicht importiert: Der Text ist größer als 1 MB, zu groß für ein Equalizer-Profil
equalizer-filter-limit =Ein Profil fasst bis zu { $max } Filter.
equalizer-paste-empty = Die Zwischenablage enthält keinen Text
equalizer-copied = Profil als Equalizer-APO-Text kopiert
equalizer-exported = Profil exportiert
equalizer-file-unreadable = Die Datei ließ sich nicht lesen
equalizer-file-unwritable = Die Datei ließ sich nicht schreiben
equalizer-value-refused = Vorheriger Wert beibehalten
equalizer-skipped-line = Zeile { $line }: { $reason }
equalizer-skipped-more = und { $count ->
        [one] eine weitere
       *[other] { $count } weitere
    }
equalizer-skip-unknown-type = unbekannter Filtertyp { $name }
equalizer-skip-unreadable = kein Filter, den Sonora lesen kann
equalizer-skip-no-frequency = keine Frequenz
equalizer-skip-no-gain = keine Verstärkung
equalizer-skip-no-width = kein Q und keine Bandbreite
equalizer-skip-expression = ein als Ausdruck geschriebener Wert
equalizer-skip-no-preamp = die Vorverstärkung ist keine Zahl
equalizer-skip-unsupported = { $name } wird nicht unterstützt
equalizer-rules = Geräteregeln
equalizer-rule-other = Jedes andere Gerät
equalizer-rule-playing = Spielt gerade
equalizer-rule-replaces = Ersetzt seine Regel
equalizer-rule-choose-device = Gerät wählen
equalizer-rule-choose-sound = Klang wählen
equalizer-rule-add = Regel hinzufügen
equalizer-rule-remove = Regel entfernen
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Die Frequenz muss zwischen { $low } und { $high } Hz liegen
equalizer-invalid-gain = Die Verstärkung muss zwischen { $low } und { $high } dB liegen
equalizer-invalid-q = Q muss zwischen { $low } und { $high } liegen
equalizer-invalid-octaves = Die Bandbreite muss zwischen { $low } und { $high } Oktaven liegen
equalizer-invalid-shelf-octaves = Ein Shelf-Filter nimmt einen Q-Wert, keine Bandbreite
equalizer-invalid-preamp = Die Vorverstärkung muss zwischen { $low } und { $high } dB liegen
equalizer-overload = Übersteuert um { $db } dB
equalizer-overload-slight = Übersteuert um weniger als 0,1 dB
equalizer-lower-preamp = Vorverstärkung senken
equalizer-level = Pegel
equalizer-status-bit-perfect = Bitgenau
equalizer-status-bit-perfect-detail = Bitgenau aus Sonora
equalizer-status-server = Der Soundserver wandelt es dann in { $khz } kHz um
equalizer-status-altered = Nicht bitgenau wegen:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Lautstärke
equalizer-status-normalized = Angeglichen
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } Bit
equalizer-status-float = Float → { $to } Bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } Kanäle
} → { $to ->
    [1] Mono
    [2] Stereo
   *[other] { $to } Kanäle
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
