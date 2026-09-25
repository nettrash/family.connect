global using Xunit;

/// <summary>
/// The tests that set the ambient CULTURE, which is process-wide: a culture switched under
/// another test's feet is a failure nobody can reproduce.
/// </summary>
[CollectionDefinition("culture", DisableParallelization = true)]
public sealed class CultureCollection;
