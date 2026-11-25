equalizer-advanced = Pokročilé
equalizer-simple = Jednoduché
equalizer-sound = Zvuk
equalizer-profile-default = Výchozí
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Vložený profil
equalizer-profile-name-hint = Název profilu
equalizer-save-as = Uložit jako profil
equalizer-rename = Přejmenovat profil
equalizer-delete = Smazat profil
equalizer-bass = Basy
equalizer-treble = Výšky
equalizer-tone-reset = Vrátit na 0 dB
equalizer-preamp = Předzesílení
equalizer-preamp-auto = Automaticky
equalizer-preamp-manual = Ručně
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = okt.
equalizer-no-gain = —
equalizer-filters = Filtry
equalizer-no-filters = Zatím žádné filtry. Přidej nějaký, vyber předvolbu výše nebo importuj profil.
equalizer-add-filter = Přidat filtr
equalizer-remove-filter = Odebrat filtr
equalizer-kind-peak = Špička
equalizer-kind-low-shelf = Dolní shelf
equalizer-kind-high-shelf = Horní shelf
equalizer-kind-low-pass = Dolní propust
equalizer-kind-high-pass = Horní propust
equalizer-kind-notch = Pásmová zádrž
equalizer-kind-band-pass = Pásmová propust
equalizer-kind-all-pass = Všepropust
equalizer-transfer = Import a export
equalizer-import = Importovat
equalizer-export = Exportovat
equalizer-import-file = Importovat soubor…
equalizer-export-file = Exportovat soubor…
equalizer-imported = Importováno: { $name }
equalizer-imported-skipped = Importováno: { $name } bez { $count ->
        [one] jednoho řádku
       *[other] { $count } řádků
    }
equalizer-import-nothing = Nenalezeno žádné nastavení ekvalizéru k importu
equalizer-import-too-many = Neimportováno: { $count ->
        [one] je zapnutý jeden filtr
        [few] jsou zapnuté { $count } filtry
       *[other] je zapnuto { $count } filtrů
    }, víc, než profil pojme
equalizer-import-too-large = Neimportováno: text má přes 1 MB, na profil ekvalizéru je příliš velký
equalizer-filter-limit =Profil pojme nejvýše { $max } filtrů.
equalizer-paste-empty = Ve schránce není žádný text
equalizer-copied = Profil zkopírován jako text Equalizer APO
equalizer-exported = Profil exportován
equalizer-file-unreadable = Soubor se nepodařilo přečíst
equalizer-file-unwritable = Soubor se nepodařilo zapsat
equalizer-value-refused = Ponechána předchozí hodnota
equalizer-skipped-line = Řádek { $line }: { $reason }
equalizer-skipped-more = a { $count ->
        [one] jeden další
        [few] { $count } další
       *[other] { $count } dalších
    }
equalizer-skip-unknown-type = neznámý typ filtru { $name }
equalizer-skip-unreadable = není filtr, který Sonora umí přečíst
equalizer-skip-no-frequency = chybí frekvence
equalizer-skip-no-gain = chybí zisk
equalizer-skip-no-width = chybí Q nebo šířka pásma
equalizer-skip-expression = hodnota zapsaná jako výraz
equalizer-skip-no-preamp = předzesílení není číslo
equalizer-skip-unsupported = { $name } není podporováno
equalizer-rules = Pravidla zařízení
equalizer-rule-other = Jakékoli jiné zařízení
equalizer-rule-playing = Právě hraje
equalizer-rule-replaces = Nahrazuje jeho pravidlo
equalizer-rule-choose-device = Vyber zařízení
equalizer-rule-choose-sound = Vyber zvuk
equalizer-rule-add = Přidat pravidlo
equalizer-rule-remove = Odebrat pravidlo
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Frekvence musí být mezi { $low } a { $high } Hz
equalizer-invalid-gain = Zisk musí být mezi { $low } a { $high } dB
equalizer-invalid-q = Q musí být mezi { $low } a { $high }
equalizer-invalid-octaves = Šířka pásma musí být mezi { $low } a { $high } oktávami
equalizer-invalid-shelf-octaves = Shelf používá Q, ne šířku pásma
equalizer-invalid-preamp = Předzesílení musí být mezi { $low } a { $high } dB
equalizer-overload = Přebuzení o { $db } dB
equalizer-overload-slight = Přebuzení pod 0,1 dB
equalizer-lower-preamp = Snížit předzesílení
equalizer-level = Úroveň
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect ze Sonory
equalizer-status-server = Zvukový server ho pak převede na { $khz } kHz
equalizer-status-altered = Není bit-perfect kvůli:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Hlasitost
equalizer-status-normalized = Normalizace
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } kan.
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } kan.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
