# track menu
menu-new-playlist = Nueva lista de reproducción
menu-import-playlist = Importar lista…
menu-no-playlists = Sin listas de reproducción
menu-search-playlists = Buscar una lista
menu-save-library = Biblioteca
menu-add-to-library = Añadir a Favoritos
menu-remove-from-library = Quitar de Favoritos
menu-remove-tracks-from-playlist = { $count ->
    [one] Quitar { $count } pista de la lista
   *[other] Quitar { $count } pistas de la lista
}
menu-remove-tracks-from-history = { $count ->
    [one] Quitar { $count } pista del historial
   *[other] Quitar { $count } pistas del historial
}
menu-download = Descargar
menu-remove-download = Quitar la descarga
menu-downloading = Descargando. Haz clic para detener
menu-song-radio = Ir a la radio de la canción
menu-go-to-album = Ir al álbum
menu-go-to-artist = Ir al artista
menu-go-to-playlist = Ir a la lista
menu-view-details = Ver detalles
menu-copy-link = Copiar enlace
menu-select-all = Seleccionar todo
menu-remove-from-queue = Quitar de la cola
menu-export-playlist = Exportar lista…
menu-add-playlist-to-library = Añadir a la biblioteca
menu-remove-playlist-from-library = Quitar de la biblioteca
menu-make-playlist-public = Hacer pública
menu-make-playlist-private = Hacer privada
menu-play-artist = Reproducir artista

# playlist editor
menu-copy-link-none = No tiene ningún enlace para compartir
menu-copy-link-local = Los artistas y las listas locales no tienen enlace
menu-play-next = { $count ->
    [1] Reproducir a continuación
    [one] Reproducir { $count } pista a continuación
   *[other] Reproducir { $count } pistas a continuación
}
menu-add-to-queue = { $count ->
    [1] Añadir a la cola
    [one] Añadir { $count } pista a la cola
   *[other] Añadir { $count } pistas a la cola
}
menu-play-last = { $count ->
    [1] Reproducir al final
    [one] Reproducir { $count } pista al final
   *[other] Reproducir { $count } pistas al final
}
menu-delete-track-files = { $count ->
    [1] Eliminar archivo de pista
   *[other] Eliminar { $count } archivos de pista
}
menu-add-to-playlist = { $count ->
    [1] Añadir a una lista
    [one] Añadir { $count } pista a una lista
   *[other] Añadir { $count } pistas a una lista
}
smart-rules-create-title = Nueva lista inteligente
smart-rules-edit-title = Reglas inteligentes
smart-rules-loading = Leyendo las reglas del servidor…
smart-rules-saving = Guardando…
smart-rules-failed-title = No se pudieron leer las reglas
smart-rules-match-all = Cumplir todas
smart-rules-match-any = Cumplir cualquiera
smart-rules-match-toggle = Cumplir todas estas reglas o cualquiera de ellas
smart-rules-of-following = de las siguientes
smart-rules-add-condition = Añadir condición
smart-rules-add-group = Añadir grupo
smart-rules-remove-group = Quitar este grupo
smart-rules-remove-condition = Quitar esta regla
smart-rules-empty-group = Aún no hay reglas aquí
smart-rules-field = Campo
smart-rules-operator = Operador
smart-rules-value-placeholder = Valor
smart-rules-range-and = y
smart-rules-true = Sí
smart-rules-false = No
smart-rules-unknown-note = Se conserva tal como lo envió el servidor
smart-rules-synced-title = Esta lista se sincroniza desde un archivo
smart-rules-synced-message = El próximo escaneo puede sobrescribir un cambio hecho aquí.
smart-rules-sort-label = Ordenar por
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = Sin dirección
smart-rules-order-asc = Ascendente
smart-rules-order-desc = Descendente
smart-rules-order-toggle = Dirección del orden
smart-rules-limit-label = Límite
smart-rules-limit-placeholder = Todas las coincidencias
smart-rules-problem-empty-group = El grupo no tiene reglas
smart-rules-problem-missing-field = La condición no indica ningún campo
smart-rules-problem-range = Un rango necesita un límite inferior y uno superior
smart-rules-problem-days = Un número de días necesita un número
smart-rules-problem-playlist = Una regla de lista necesita una lista
smart-rules-problem-truth = La regla necesita un valor verdadero o falso
smart-rules-problem-compared = El valor no encaja con la regla
smart-rules-problem-boolean = Un campo booleano necesita un valor verdadero o falso
smart-rules-problem-bad-order = El orden es asc o desc

# playlist editor
playlist-name-placeholder = Nombre de la lista
playlist-create-title = Crear lista de reproducción
playlist-create-shelf = Guardar en
playlist-edit-title = Editar lista
playlist-cover-choose = Elegir imagen
playlist-cover-remove = Quitar portada
playlist-cover-unopenable = Sonora no puede abrir este archivo
playlist-cover-too-large = Esta imagen pesa más de 40 MB
playlist-cover-not-a-picture = Este archivo no es una imagen JPEG, PNG ni WebP
playlist-delete-title = Eliminar lista de reproducción
playlist-again-title = ¿Añadirla otra vez?
playlist-again-confirm = Esta pista ya está en “{ $name }”. ¿Quieres añadir otra copia?
playlist-again-add = Añadir otra vez
cover-view = Ver portada
cover-zoom-in = Acercar
cover-zoom-out = Alejar
cover-zoom-fit = Ajustar a la ventana

# confirm

# confirm
confirm-delete-track-files-title = ¿Eliminar los archivos de las pistas?
confirm-delete-track-files = El archivo o los archivos se eliminarán del disco de forma permanente. Esta acción no se puede deshacer.
confirm-uninstall-widevine-title = ¿Desinstalar el módulo Widevine?
confirm-uninstall-widevine = Sonora eliminará la copia que descargó de Google. Las pistas de Apple Music dejarán de reproducirse hasta que se vuelva a descargar.
confirm-remove-soloist-title = ¿Quitar Soloist?
confirm-remove-soloist = Sonora borra el programa Soloist, tu clave de API y su inicio de sesión. Spotify vuelve al reproductor integrado.
confirm-remove-downloads-title = ¿Eliminar todas las descargas?
confirm-remove-downloads = Todas las canciones que descargaste se eliminarán de este equipo, de todos los servicios. Volverán a reproducirse en streaming hasta que las descargues de nuevo.
confirm-delete-named = ¿Eliminar “{ $name }”? Esta acción no se puede deshacer.
