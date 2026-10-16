package ru.pchelopoisk.app

object TrustedNodes {
    val hives = listOf("https://45.158.46.52")
    fun hive(): String = hives.first()
}
