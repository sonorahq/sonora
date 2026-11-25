equalizer-advanced = Avanzato
equalizer-simple = Semplice
equalizer-sound = Suono
equalizer-profile-default = Predefinito
equalizer-profile-new = Profilo { $number }
equalizer-profile-pasted = Profilo incollato
equalizer-profile-name-hint = Nome profilo
equalizer-save-as = Salva come profilo
equalizer-rename = Rinomina profilo
equalizer-delete = Elimina profilo
equalizer-bass = Bassi
equalizer-treble = Alti
equalizer-tone-reset = Reimposta a 0 dB
equalizer-preamp = Preamp
equalizer-preamp-auto = Automatico
equalizer-preamp-manual = Manuale
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = ott
equalizer-no-gain = —
equalizer-filters = Filtri
equalizer-no-filters = Ancora nessun filtro. Aggiungine uno, scegli un preset qui sopra o importa un profilo.
equalizer-add-filter = Aggiungi filtro
equalizer-remove-filter = Rimuovi filtro
equalizer-kind-peak = Picco
equalizer-kind-low-shelf = Shelf basso
equalizer-kind-high-shelf = Shelf alto
equalizer-kind-low-pass = Passa-basso
equalizer-kind-high-pass = Passa-alto
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Passa-banda
equalizer-kind-all-pass = Passa-tutto
equalizer-transfer = Importa ed esporta
equalizer-import = Importa
equalizer-export = Esporta
equalizer-import-file = Importa file…
equalizer-export-file = Esporta file…
equalizer-imported = { $name } importato
equalizer-imported-skipped = { $name } importato senza { $count ->
        [one] una riga
       *[other] { $count } righe
    }
equalizer-import-nothing = Nessuna impostazione dell'equalizzatore da importare
equalizer-import-too-many = Non importato: { $count ->
        [one] è attivo un filtro
       *[other] sono attivi { $count } filtri
    }, più di quanti ne contenga un profilo
equalizer-import-too-large = Non importato: il testo supera 1 MB, troppo per un profilo dell'equalizzatore
equalizer-filter-limit =Un profilo contiene fino a { $max } filtri.
equalizer-paste-empty = Gli appunti non contengono testo
equalizer-copied = Profilo copiato come testo Equalizer APO
equalizer-exported = Profilo esportato
equalizer-file-unreadable = Impossibile leggere il file
equalizer-file-unwritable = Impossibile scrivere il file
equalizer-value-refused = Mantenuto il valore precedente
equalizer-skipped-line = Riga { $line }: { $reason }
equalizer-skipped-more = e { $count ->
        [one] un'altra
       *[other] altre { $count }
    }
equalizer-skip-unknown-type = tipo di filtro sconosciuto { $name }
equalizer-skip-unreadable = non è un filtro che Sonora può leggere
equalizer-skip-no-frequency = nessuna frequenza
equalizer-skip-no-gain = nessun guadagno
equalizer-skip-no-width = nessun Q o larghezza di banda
equalizer-skip-expression = un valore scritto come espressione
equalizer-skip-no-preamp = il preamp non è un numero
equalizer-skip-unsupported = { $name } non è supportato
equalizer-rules = Regole per dispositivo
equalizer-rule-other = Qualsiasi altro dispositivo
equalizer-rule-playing = In riproduzione
equalizer-rule-replaces = Sostituisce la sua regola
equalizer-rule-choose-device = Scegli un dispositivo
equalizer-rule-choose-sound = Scegli un suono
equalizer-rule-add = Aggiungi regola
equalizer-rule-remove = Rimuovi regola
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = La frequenza deve essere tra { $low } e { $high } Hz
equalizer-invalid-gain = Il guadagno deve essere tra { $low } e { $high } dB
equalizer-invalid-q = Il Q deve essere tra { $low } e { $high }
equalizer-invalid-octaves = La larghezza di banda deve essere tra { $low } e { $high } ottave
equalizer-invalid-shelf-octaves = Uno shelf usa un Q, non una larghezza di banda
equalizer-invalid-preamp = Il preamp deve essere tra { $low } e { $high } dB
equalizer-overload = Clipping di { $db } dB
equalizer-overload-slight = Clipping inferiore a 0,1 dB
equalizer-lower-preamp = Abbassa il preamp
equalizer-level = Livello
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect da Sonora
equalizer-status-server = Il server audio poi lo converte a { $khz } kHz
equalizer-status-altered = Non bit-perfect a causa di:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Normalizzato
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } can.
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } can.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
