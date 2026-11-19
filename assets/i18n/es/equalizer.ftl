equalizer-advanced = Avanzado
equalizer-simple = Sencillo
equalizer-sound = Sonido
equalizer-profile-default = Predeterminado
equalizer-profile-new = Perfil { $number }
equalizer-profile-pasted = Perfil pegado
equalizer-profile-name-hint = Nombre del perfil
equalizer-save-as = Guardar como perfil
equalizer-rename = Renombrar perfil
equalizer-delete = Eliminar perfil
equalizer-bass = Graves
equalizer-treble = Agudos
equalizer-tone-reset = Restablecer a 0 dB
equalizer-preamp = Preamp
equalizer-preamp-auto = Automático
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct.
equalizer-no-gain = —
equalizer-filters = Filtros
equalizer-no-filters = Todavía no hay filtros. Añade uno, elige un preajuste arriba o importa un perfil.
equalizer-add-filter = Añadir filtro
equalizer-remove-filter = Quitar filtro
equalizer-kind-peak = Pico
equalizer-kind-low-shelf = Shelf de graves
equalizer-kind-high-shelf = Shelf de agudos
equalizer-kind-low-pass = Paso bajo
equalizer-kind-high-pass = Paso alto
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Paso banda
equalizer-kind-all-pass = Paso todo
equalizer-transfer = Importar y exportar
equalizer-import = Importar
equalizer-export = Exportar
equalizer-import-file = Importar archivo…
equalizer-export-file = Exportar archivo…
equalizer-imported = Se importó { $name }
equalizer-imported-skipped = Se importó { $name } sin { $count ->
        [one] una línea
       *[other] { $count } líneas
    }
equalizer-import-nothing = No se encontraron ajustes de ecualizador para importar
equalizer-import-too-many = No se importó: { $count ->
        [one] hay un filtro activo
       *[other] hay { $count } filtros activos
    }, más de los que admite un perfil
equalizer-import-too-large = No se importó: el texto supera 1 MB, demasiado para un perfil de ecualizador
equalizer-filter-limit = Un perfil admite hasta { $max } filtros.
equalizer-paste-empty = El portapapeles no contiene texto
equalizer-copied = Perfil copiado como texto de Equalizer APO
equalizer-exported = Perfil exportado
equalizer-file-unreadable = No se pudo leer el archivo
equalizer-file-unwritable = No se pudo escribir el archivo
equalizer-value-refused = Se mantuvo el valor anterior
equalizer-skipped-line = Línea { $line }: { $reason }
equalizer-skipped-more = y { $count ->
        [one] una más
       *[other] { $count } más
    }
equalizer-skip-unknown-type = tipo de filtro desconocido { $name }
equalizer-skip-unreadable = no es un filtro que Sonora pueda leer
equalizer-skip-no-frequency = sin frecuencia
equalizer-skip-no-gain = sin ganancia
equalizer-skip-no-width = sin Q ni ancho de banda
equalizer-skip-expression = un valor escrito como expresión
equalizer-skip-no-preamp = el preamp no es un número
equalizer-skip-unsupported = { $name } no es compatible
equalizer-rules = Reglas por dispositivo
equalizer-rule-other = Cualquier otro dispositivo
equalizer-rule-playing = Sonando ahora
equalizer-rule-replaces = Sustituye su regla
equalizer-rule-choose-device = Elige un dispositivo
equalizer-rule-choose-sound = Elige un sonido
equalizer-rule-add = Añadir regla
equalizer-rule-remove = Quitar regla
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = La frecuencia debe estar entre { $low } y { $high } Hz
equalizer-invalid-gain = La ganancia debe estar entre { $low } y { $high } dB
equalizer-invalid-q = Q debe estar entre { $low } y { $high }
equalizer-invalid-octaves = El ancho de banda debe estar entre { $low } y { $high } octavas
equalizer-invalid-shelf-octaves = Un shelf usa Q, no ancho de banda
equalizer-invalid-preamp = El preamp debe estar entre { $low } y { $high } dB
equalizer-overload = Saturación de { $db } dB
equalizer-overload-slight = Saturación de menos de 0,1 dB
equalizer-lower-preamp = Bajar el preamp
equalizer-level = Nivel
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect desde Sonora
equalizer-status-server = Luego el servidor de sonido lo convierte a { $khz } kHz
equalizer-status-altered = No es bit-perfect debido a:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volumen
equalizer-status-normalized = Normalizado
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bits
equalizer-status-float = Flotante → { $to } bits
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Estéreo
   *[other] { $from } can.
} → { $to ->
    [1] mono
    [2] estéreo
   *[other] { $to } can.
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
